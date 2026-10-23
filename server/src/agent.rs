use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::time::Duration;

pub fn router() -> Router {
    Router::new().route("/connect", get(connect))
}

async fn connect(
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    let expected = std::env::var("TUNNELHUB_AGENT_TOKEN")
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if expected.len() < 24 {
        tracing::error!("TUNNELHUB_AGENT_TOKEN must contain at least 24 characters");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let supplied = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !constant_time_eq(supplied.as_bytes(), expected.as_bytes()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(ws.on_upgrade(handle_socket))
}

#[derive(Deserialize)]
struct AgentMessage {
    #[serde(rename = "type")]
    kind: String,
    agent_id: Option<String>,
    version: Option<String>,
}

async fn handle_socket(socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let Some(Ok(Message::Text(first))) = receiver.next().await else {
        return;
    };
    let Ok(hello) = serde_json::from_str::<AgentMessage>(&first) else {
        let _ = sender.send(Message::Close(None)).await;
        return;
    };
    if hello.kind != "hello" {
        let _ = sender.send(Message::Close(None)).await;
        return;
    }
    let agent_id = hello.agent_id.unwrap_or_else(|| "unnamed".to_string());
    tracing::info!(%agent_id, version = hello.version.as_deref().unwrap_or("unknown"), "Agent connected");
    if sender
        .send(Message::Text(
            serde_json::json!({"type":"welcome","protocol_version":1}).to_string().into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    let mut heartbeat_deadline = tokio::time::interval(Duration::from_secs(45));
    heartbeat_deadline.tick().await;
    loop {
        tokio::select! {
            _ = heartbeat_deadline.tick() => {
                tracing::warn!(%agent_id, "Agent heartbeat timeout");
                break;
            }
            message = receiver.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<AgentMessage>(&text) {
                            Ok(message) if message.kind == "heartbeat" => {
                                heartbeat_deadline.reset();
                                if sender.send(Message::Text(
                                    serde_json::json!({"type":"heartbeat_ack"}).to_string().into()
                                )).await.is_err() {
                                    break;
                                }
                            }
                            Ok(message) if message.kind == "goodbye" => break,
                            Ok(_) => {}
                            Err(error) => tracing::warn!(%agent_id, ?error, "Invalid Agent message"),
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
    tracing::info!(%agent_id, "Agent disconnected");
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}
