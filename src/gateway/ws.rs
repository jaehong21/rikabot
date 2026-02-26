use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::agent::AgentEvent;
use crate::gateway::AppState;
use crate::providers::ChatMessage;

// ── WebSocket messages ──────────────────────────────────────────────────────

/// Inbound message from the client.
#[derive(Debug, Deserialize)]
struct ClientMessage {
    #[serde(rename = "type")]
    msg_type: String,
    content: Option<String>,
}

// ── Handler ─────────────────────────────────────────────────────────────────

/// Axum handler that upgrades an HTTP request to a WebSocket connection.
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle a single WebSocket connection.
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Per-connection conversation history
    let mut history: Vec<ChatMessage> = Vec::new();

    while let Some(Ok(msg)) = ws_stream.next().await {
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            _ => continue,
        };

        // Parse client message
        let client_msg: ClientMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                let err = serde_json::json!({
                    "type": "error",
                    "message": format!("Invalid message format: {}", e),
                });
                let _ = ws_sink.send(Message::text(err.to_string())).await;
                continue;
            }
        };

        if client_msg.msg_type != "message" {
            continue;
        }

        let content = match client_msg.content {
            Some(c) if !c.trim().is_empty() => c,
            _ => continue,
        };

        // Create a channel for agent events
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();

        // Spawn the agent loop
        let agent = state.agent.clone();
        let mut history_clone = history.clone();
        let content_clone = content.clone();

        let agent_handle = tokio::spawn(async move {
            let result = agent.run(&mut history_clone, content_clone, event_tx).await;
            (result, history_clone)
        });

        // Forward agent events to WebSocket
        while let Some(event) = event_rx.recv().await {
            let json = match serde_json::to_string(&event) {
                Ok(j) => j,
                Err(_) => continue,
            };
            if ws_sink.send(Message::text(json)).await.is_err() {
                break;
            }
        }

        // Wait for the agent to finish and update history
        match agent_handle.await {
            Ok((Ok(()), updated_history)) => {
                history = updated_history;
            }
            Ok((Err(e), _)) => {
                let err = serde_json::json!({
                    "type": "error",
                    "message": format!("Agent error: {}", e),
                });
                let _ = ws_sink.send(Message::text(err.to_string())).await;
            }
            Err(e) => {
                let err = serde_json::json!({
                    "type": "error",
                    "message": format!("Internal error: {}", e),
                });
                let _ = ws_sink.send(Message::text(err.to_string())).await;
            }
        }
    }

    tracing::debug!("WebSocket connection closed");
}
