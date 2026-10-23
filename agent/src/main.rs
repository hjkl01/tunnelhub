use std::{env, time::Duration};

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::header::AUTHORIZATION, Message},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use url::Url;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "tunnelhub_agent=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let server = env::var("TUNNELHUB_SERVER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let token = env::var("TUNNELHUB_AGENT_TOKEN").unwrap_or_default();
    if token.len() < 24 {
        bail!("TUNNELHUB_AGENT_TOKEN is required and must contain at least 24 characters");
    }

    let agent_id = env::var("TUNNELHUB_AGENT_ID")
        .unwrap_or_else(|_| "tunnelhub-agent".to_string());
    let version = env!("CARGO_PKG_VERSION");
    let mut retry_delay = Duration::from_secs(1);

    loop {
        match connect_and_run(&server, &token, &agent_id, version).await {
            Ok(()) => tracing::warn!("Server closed the Agent connection"),
            Err(error) => tracing::warn!(?error, "Agent connection failed"),
        }
        tracing::info!(?retry_delay, "Reconnecting to TunnelHub");
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
    }
}

async fn connect_and_run(
    server: &str,
    token: &str,
    agent_id: &str,
    version: &str,
) -> anyhow::Result<()> {
    let mut url = Url::parse(server).context("invalid TUNNELHUB_SERVER_URL")?;
    url.set_scheme(match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        "ws" => "ws",
        "wss" => "wss",
        scheme => bail!("unsupported server URL scheme: {scheme}"),
    }).map_err(|_| anyhow::anyhow!("could not convert server URL to WebSocket URL"))?;
    url.set_path("/api/agent/connect");
    url.set_query(None);
    url.set_fragment(None);

    let mut request = url.as_str().into_client_request()?;
    request.headers_mut().insert(
        AUTHORIZATION,
        format!("Bearer {token}").parse().context("invalid agent token header")?,
    );
    tracing::info!(endpoint = %url, %agent_id, "Connecting to TunnelHub");
    let (socket, _) = connect_async(request).await.context("WebSocket handshake failed")?;
    let (mut sender, mut receiver) = socket.split();

    sender.send(Message::Text(serde_json::json!({
        "type": "hello",
        "agent_id": agent_id,
        "version": version,
        "protocol_version": 1
    }).to_string().into())).await?;

    let welcome = tokio::time::timeout(Duration::from_secs(10), receiver.next())
        .await
        .context("timed out waiting for server welcome")?
        .context("server closed before welcome")??;
    let Message::Text(welcome) = welcome else {
        bail!("server sent an unexpected welcome frame");
    };
    let welcome: serde_json::Value = serde_json::from_str(&welcome)?;
    if welcome["type"] != "welcome" || welcome["protocol_version"] != 1 {
        bail!("server returned an incompatible protocol handshake: {welcome}");
    }

    tracing::info!(%agent_id, "Agent tunnel connected");
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                sender.send(Message::Text(serde_json::json!({
                    "type": "heartbeat",
                    "agent_id": agent_id,
                }).to_string().into())).await?;
            }
            message = receiver.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        let value: serde_json::Value = serde_json::from_str(&text)?;
                        match value["type"].as_str() {
                            Some("heartbeat_ack") => tracing::trace!(%agent_id, "Heartbeat acknowledged"),
                            Some("close") => bail!("server requested Agent shutdown"),
                            Some(other) => tracing::debug!(%agent_id, message_type = other, "Received server message"),
                            None => tracing::debug!(%agent_id, "Received non-protocol text message"),
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => sender.send(Message::Pong(payload)).await?,
                    Some(Ok(Message::Close(frame))) => {
                        tracing::info!(?frame, %agent_id, "Server closed WebSocket");
                        return Ok(());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => return Err(error.into()),
                    None => return Ok(()),
                }
            }
        }
    }
}
