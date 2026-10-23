use std::{env, time::Duration};

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "tunnelhub_agent=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let server = env::var("TUNNELHUB_SERVER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let token = env::var("TUNNELHUB_AGENT_TOKEN").unwrap_or_default();

    if token.is_empty() {
        anyhow::bail!("TUNNELHUB_AGENT_TOKEN is required; obtain a node token from the server administrator");
    }

    tracing::info!(%server, "TunnelHub Agent starting");
    tracing::warn!("Agent transport is not implemented yet; this binary currently validates configuration only");
    let _ = token;
    tokio::time::sleep(Duration::from_millis(1)).await;
    anyhow::bail!("agent transport is not implemented yet")
}
