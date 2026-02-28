use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::agent::{AgentEvent, ToolApprovalDecision, ToolApprovalDecisionKind};
use crate::config::{PermissionsConfig, ToolPermissionsConfig};
use crate::gateway::{ActiveRunState, AppState};
use crate::mcp_runtime::McpStatusSnapshot;
use crate::permissions::PermissionEngine;
use crate::providers::ChatMessage;

/// Inbound message from the client.
#[derive(Debug, Clone, Deserialize)]
struct ClientMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    allow: Option<Vec<String>>,
    #[serde(default)]
    deny: Option<Vec<String>>,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    decision: Option<ToolApprovalDecisionKind>,
    #[serde(default)]
    allow_rule: Option<String>,
}

struct RunOutcome {
    result: anyhow::Result<()>,
    updated_history: Vec<ChatMessage>,
    previous_len: usize,
}

/// Axum handler that upgrades an HTTP request to a WebSocket connection.
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle a single WebSocket connection.
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_sink, mut ws_stream) = socket.split();
    let (run_signal_tx, mut run_signal_rx) = mpsc::unbounded_channel::<Value>();
    let mut mcp_status_rx = state.mcp_runtime.subscribe();

    let (mut current_session_id, mut history) = match hydrate_current_thread(&state).await {
        Ok((sid, initial_history)) => {
            if send_thread_list(&mut ws_sink, &state).await.is_err() {
                return;
            }
            if send_thread_switched(&mut ws_sink, &sid, &initial_history)
                .await
                .is_err()
            {
                return;
            }
            if send_permissions_state(&mut ws_sink, &state, None)
                .await
                .is_err()
            {
                return;
            }
            if send_mcp_status(&mut ws_sink, state.mcp_runtime.snapshot())
                .await
                .is_err()
            {
                return;
            }
            (sid, initial_history)
        }
        Err(err) => {
            let _ = send_error(
                &mut ws_sink,
                &format!("Failed to initialize session: {}", err),
            )
            .await;
            return;
        }
    };

    if let Err(err) = attach_to_active_run(
        &state,
        &current_session_id,
        run_signal_tx.clone(),
        &mut ws_sink,
    )
    .await
    {
        let _ = send_error(
            &mut ws_sink,
            &format!("Failed to reattach active run: {}", err),
        )
        .await;
        return;
    }

    loop {
        tokio::select! {
            ws_item = ws_stream.next() => {
                let msg = match ws_item {
                    Some(Ok(msg)) => msg,
                    Some(Err(err)) => {
                        tracing::debug!("WebSocket read error: {}", err);
                        break;
                    }
                    None => break,
                };

                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    _ => continue,
                };

                let client_msg: ClientMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = send_error(&mut ws_sink, &format!("Invalid message format: {}", e)).await;
                        continue;
                    }
                };

                match client_msg.msg_type.as_str() {
                    "message" => {
                        if has_active_run(&state).await {
                            let _ = send_error(&mut ws_sink, "A run is already active. Stop it before sending another message.").await;
                            continue;
                        }

                        let content = match client_msg.content {
                            Some(c) if !c.trim().is_empty() => c,
                            _ => continue,
                        };

                        let base_history = match load_history_for_session(&state, &current_session_id).await {
                            Ok(history) => history,
                            Err(err) => {
                                let _ = send_error(
                                    &mut ws_sink,
                                    &format!("Failed to load session history: {}", err),
                                )
                                .await;
                                continue;
                            }
                        };

                        match spawn_active_run(
                            &state,
                            run_signal_tx.clone(),
                            &current_session_id,
                            &base_history,
                            content,
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(err) => {
                                let _ = send_error(
                                    &mut ws_sink,
                                    &format!("Failed to start agent run: {}", err),
                                )
                                .await;
                            }
                        }
                    }
                    "kill_switch" => {
                        if stop_active_run(&state, "user_cancelled", Some(&current_session_id)).await {
                        } else {
                            let _ = send_stopped(&mut ws_sink, "no_active_run", Some(&current_session_id)).await;
                        }
                    }
                    "permissions_get" => {
                        if let Err(err) = send_permissions_state(&mut ws_sink, &state, None).await {
                            tracing::debug!("Failed to send permissions state: {}", err);
                            break;
                        }
                    }
                    "permissions_set" => {
                        if let Err(err) = handle_permissions_set(&state, &client_msg, &mut ws_sink).await {
                            let _ = send_error(
                                &mut ws_sink,
                                &format!("Failed to update permissions: {}", err),
                            )
                            .await;
                        }
                    }
                    "tool_approval_decision" => {
                        if let Err(err) =
                            handle_tool_approval_decision(&state, &client_msg, &mut ws_sink).await
                        {
                            let _ = send_error(
                                &mut ws_sink,
                                &format!("Failed to apply tool approval decision: {}", err),
                            )
                            .await;
                        }
                    }
                    _ => {
                        if has_active_run(&state).await && is_thread_mutating_command(&client_msg.msg_type) {
                            let _ = send_error(
                                &mut ws_sink,
                                "Cannot modify threads while a run is active. Stop the run first.",
                            )
                            .await;
                            continue;
                        }

                        match handle_thread_command(
                            &state,
                            &client_msg,
                            &mut current_session_id,
                            &mut history,
                        )
                        .await
                        {
                            Ok(Some(event)) => {
                                let switched_to = event
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .filter(|t| *t == "thread_switched")
                                    .and_then(|_| event.get("session_id"))
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string);

                                if ws_sink
                                    .send(Message::text(event.to_string()))
                                    .await
                                .is_err()
                                {
                                    break;
                                }

                                if let Some(switched_session_id) = switched_to {
                                    if let Err(err) = attach_to_active_run(
                                        &state,
                                        &switched_session_id,
                                        run_signal_tx.clone(),
                                        &mut ws_sink,
                                    )
                                    .await
                                    {
                                        let _ = send_error(
                                            &mut ws_sink,
                                            &format!(
                                                "Failed to attach run after thread switch: {}",
                                                err
                                            ),
                                        )
                                        .await;
                                        break;
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(err) => {
                                let _ = send_error(&mut ws_sink, &err.to_string()).await;
                            }
                        }
                    }
                }
            }
            signal = run_signal_rx.recv() => {
                let Some(payload) = signal else {
                    break;
                };

                if ws_sink
                    .send(Message::text(payload.to_string()))
                    .await
                .is_err()
                {
                    break;
                }
            }
            status_update = mcp_status_rx.changed() => {
                if status_update.is_err() {
                    break;
                }
                let snapshot = mcp_status_rx.borrow().clone();
                if send_mcp_status(&mut ws_sink, snapshot).await.is_err() {
                    break;
                }
            }
        }
    }

    tracing::debug!("WebSocket connection closed");
}

async fn spawn_active_run(
    state: &AppState,
    run_signal_tx: mpsc::UnboundedSender<Value>,
    session_id: &str,
    history: &[ChatMessage],
    content: String,
) -> anyhow::Result<()> {
    let system_prompt = state.prompt_manager.build_prompt()?;
    let mut runs_guard = state.runs.lock().await;
    if runs_guard.active.is_some() {
        return Err(anyhow::anyhow!(
            "A run is already active. Stop it before sending another message."
        ));
    }

    let run_id = runs_guard.next_run_id;
    runs_guard.next_run_id = runs_guard.next_run_id.saturating_add(1);

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (approval_tx, approval_rx) = mpsc::unbounded_channel::<ToolApprovalDecision>();
    let (outcome_tx, outcome_rx) = oneshot::channel::<RunOutcome>();
    let agent = state.agent.clone();
    let sessions = state.sessions.clone();
    let run_store = state.runs.clone();
    let mut history_clone = history.to_vec();
    let previous_len = history_clone.len();
    let content_for_event = content.clone();
    let session_id_owned = session_id.to_string();

    let agent_task = tokio::spawn(async move {
        let result = agent
            .run(
                system_prompt,
                &mut history_clone,
                content,
                event_tx,
                approval_rx,
            )
            .await;
        let _ = outcome_tx.send(RunOutcome {
            result,
            updated_history: history_clone,
            previous_len,
        });
    });

    let session_id_for_task = session_id_owned.clone();
    let event_task = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if let AgentEvent::ToolApprovalRequired { request_id, .. } = &event {
                register_pending_approval(&run_store, run_id, request_id).await;
            }
            if let AgentEvent::ToolCallResult {
                approval_request_id: Some(request_id),
                awaiting_approval: false,
                ..
            } = &event
            {
                clear_pending_approval(&run_store, run_id, request_id).await;
            }

            let payload = match serde_json::to_value(&event) {
                Ok(value) => value,
                Err(_) => continue,
            };
            broadcast_run_payload(&run_store, run_id, payload).await;
        }

        if let Ok(outcome) = outcome_rx.await {
            if matches!(outcome.result, Ok(()))
                && outcome.previous_len <= outcome.updated_history.len()
            {
                let appended = &outcome.updated_history[outcome.previous_len..];
                let append_result = {
                    let mut sessions = sessions.lock().await;
                    sessions.append_messages(&session_id_for_task, appended)
                };
                if let Err(err) = append_result {
                    let payload = serde_json::json!({
                        "type": "error",
                        "message": format!("Failed to persist session messages: {}", err),
                    });
                    broadcast_run_payload(&run_store, run_id, payload).await;
                } else {
                    let thread_list_payload = {
                        let sessions = sessions.lock().await;
                        serde_json::json!({
                            "type": "thread_list",
                            "current_session_id": sessions.current_session_id(),
                            "sessions": sessions.list_sessions(),
                        })
                    };
                    broadcast_run_payload(&run_store, run_id, thread_list_payload).await;
                }
            }
        }

        clear_active_run(&run_store, run_id).await;
    });

    let user_payload = serde_json::json!({
        "type": "user_message",
        "content": content_for_event,
    });
    let _ = run_signal_tx.send(user_payload.clone());
    runs_guard.active = Some(ActiveRunState {
        run_id,
        session_id: session_id_owned,
        events: vec![user_payload],
        subscribers: vec![run_signal_tx],
        approval_tx,
        pending_approval_ids: std::collections::HashSet::new(),
        agent_task,
        event_task,
    });

    Ok(())
}

async fn broadcast_run_payload(
    runs: &std::sync::Arc<tokio::sync::Mutex<crate::gateway::RunManager>>,
    run_id: u64,
    payload: Value,
) {
    let mut runs = runs.lock().await;
    let Some(active) = runs.active.as_mut() else {
        return;
    };
    if active.run_id != run_id {
        return;
    }

    active.events.push(payload.clone());
    active
        .subscribers
        .retain(|sub| sub.send(payload.clone()).is_ok());
}

async fn register_pending_approval(
    runs: &std::sync::Arc<tokio::sync::Mutex<crate::gateway::RunManager>>,
    run_id: u64,
    request_id: &str,
) {
    let mut runs = runs.lock().await;
    let Some(active) = runs.active.as_mut() else {
        return;
    };
    if active.run_id != run_id {
        return;
    }
    active.pending_approval_ids.insert(request_id.to_string());
}

async fn clear_pending_approval(
    runs: &std::sync::Arc<tokio::sync::Mutex<crate::gateway::RunManager>>,
    run_id: u64,
    request_id: &str,
) {
    let mut runs = runs.lock().await;
    let Some(active) = runs.active.as_mut() else {
        return;
    };
    if active.run_id != run_id {
        return;
    }
    active.pending_approval_ids.remove(request_id);
}

async fn clear_active_run(
    runs: &std::sync::Arc<tokio::sync::Mutex<crate::gateway::RunManager>>,
    run_id: u64,
) {
    let mut runs = runs.lock().await;
    let remove = runs.active.as_ref().is_some_and(|run| run.run_id == run_id);
    if remove {
        runs.active = None;
    }
}

async fn load_history_for_session(
    state: &AppState,
    session_id: &str,
) -> anyhow::Result<Vec<ChatMessage>> {
    let mut sessions = state.sessions.lock().await;
    sessions.load_history(session_id)
}

async fn has_active_run(state: &AppState) -> bool {
    let runs = state.runs.lock().await;
    runs.active.is_some()
}

async fn attach_to_active_run(
    state: &AppState,
    session_id: &str,
    run_signal_tx: mpsc::UnboundedSender<Value>,
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    let replay = {
        let mut runs = state.runs.lock().await;
        let Some(active) = runs.active.as_mut() else {
            return Ok(());
        };
        if active.session_id != session_id {
            return Ok(());
        }
        active.subscribers.push(run_signal_tx);
        active.events.clone()
    };

    for payload in replay {
        ws_sink.send(Message::text(payload.to_string())).await?;
    }

    Ok(())
}

async fn stop_active_run(state: &AppState, reason: &str, session_id: Option<&str>) -> bool {
    let run = {
        let mut runs = state.runs.lock().await;
        let Some(active) = runs.active.take() else {
            return false;
        };
        if let Some(requested_sid) = session_id {
            if active.session_id != requested_sid {
                runs.active = Some(active);
                return false;
            }
        }
        active
    };

    let payload = serde_json::json!({
        "type": "stopped",
        "reason": reason,
        "session_id": run.session_id,
    });
    for sub in &run.subscribers {
        let _ = sub.send(payload.clone());
    }

    run.agent_task.abort();
    run.event_task.abort();
    let _ = run.agent_task.await;
    let _ = run.event_task.await;
    true
}

async fn hydrate_current_thread(state: &AppState) -> anyhow::Result<(String, Vec<ChatMessage>)> {
    let mut sessions = state.sessions.lock().await;
    let sid = sessions.current_session_id().to_string();
    let history = sessions.load_history(&sid)?;
    Ok((sid, history))
}

async fn send_thread_list(
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
) -> anyhow::Result<()> {
    let payload = {
        let sessions = state.sessions.lock().await;
        serde_json::json!({
            "type": "thread_list",
            "current_session_id": sessions.current_session_id(),
            "sessions": sessions.list_sessions(),
        })
    };
    ws_sink
        .send(Message::text(payload.to_string()))
        .await
        .map_err(Into::into)
}

async fn send_thread_switched(
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    session_id: &str,
    history: &[ChatMessage],
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "type": "thread_switched",
        "session_id": session_id,
        "history": history,
    });
    ws_sink
        .send(Message::text(payload.to_string()))
        .await
        .map_err(Into::into)
}

async fn send_error(
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: &str,
) -> anyhow::Result<()> {
    ws_sink
        .send(Message::text(
            serde_json::json!({"type": "error", "message": message}).to_string(),
        ))
        .await
        .map_err(Into::into)
}

async fn send_stopped(
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    reason: &str,
    session_id: Option<&str>,
) -> anyhow::Result<()> {
    ws_sink
        .send(Message::text(
            serde_json::json!({
                "type": "stopped",
                "reason": reason,
                "session_id": session_id,
            })
            .to_string(),
        ))
        .await
        .map_err(Into::into)
}

async fn send_permissions_state(
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    validation_errors: Option<Vec<String>>,
) -> anyhow::Result<()> {
    let permissions = {
        let current = state.permissions_config.read().await;
        current.clone()
    };

    let payload = serde_json::json!({
        "type": "permissions_state",
        "permissions": permissions,
        "validation_errors": validation_errors.unwrap_or_default(),
    });

    ws_sink
        .send(Message::text(payload.to_string()))
        .await
        .map_err(Into::into)
}

async fn send_permissions_updated(
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    permissions: &PermissionsConfig,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "type": "permissions_updated",
        "permissions": permissions,
    });

    ws_sink
        .send(Message::text(payload.to_string()))
        .await
        .map_err(Into::into)
}

async fn send_mcp_status(
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    snapshot: McpStatusSnapshot,
) -> anyhow::Result<()> {
    let payload = serde_json::json!({
        "type": "mcp_status",
        "mcp": snapshot,
    });

    ws_sink
        .send(Message::text(payload.to_string()))
        .await
        .map_err(Into::into)
}

async fn handle_permissions_set(
    state: &AppState,
    client_msg: &ClientMessage,
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    let enabled = client_msg
        .enabled
        .ok_or_else(|| anyhow::anyhow!("missing required field 'enabled'"))?;
    let allow = sanitize_rules(client_msg.allow.as_ref());
    let deny = sanitize_rules(client_msg.deny.as_ref());

    let next = PermissionsConfig {
        enabled,
        tools: ToolPermissionsConfig { allow, deny },
    };

    if let Err(err) = apply_permissions_update(state, next.clone()).await {
        send_permissions_state(ws_sink, state, Some(vec![err.to_string()])).await?;
        return Ok(());
    }

    send_permissions_updated(ws_sink, &next).await
}

async fn handle_tool_approval_decision(
    state: &AppState,
    client_msg: &ClientMessage,
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    let request_id = required_field(client_msg.request_id.as_deref(), "request_id")?.to_string();
    let decision = client_msg
        .decision
        .clone()
        .ok_or_else(|| anyhow::anyhow!("missing required field 'decision'"))?;

    if decision == ToolApprovalDecisionKind::AllowPersist {
        let allow_rule = required_field(client_msg.allow_rule.as_deref(), "allow_rule")?;
        persist_allow_rule_from_approval(state, allow_rule, ws_sink).await?;
    }

    let approval_tx = {
        let mut runs = state.runs.lock().await;
        let Some(active) = runs.active.as_mut() else {
            anyhow::bail!("no active run for tool approval decision");
        };
        if !active.pending_approval_ids.remove(&request_id) {
            anyhow::bail!(
                "unknown or expired tool approval request_id '{}'",
                request_id
            );
        }
        active.approval_tx.clone()
    };

    approval_tx
        .send(ToolApprovalDecision {
            request_id,
            decision,
        })
        .map_err(|_| anyhow::anyhow!("active run is not accepting approval decisions"))?;

    Ok(())
}

async fn persist_allow_rule_from_approval(
    state: &AppState,
    allow_rule: &str,
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    let allow_rule = allow_rule.trim();
    if allow_rule.is_empty() {
        anyhow::bail!("allow_rule cannot be empty");
    }

    let next = {
        let current = state.permissions_config.read().await;
        let mut next = current.clone();
        if !next.tools.allow.iter().any(|rule| rule == allow_rule) {
            next.tools.allow.push(allow_rule.to_string());
        }
        next
    };

    apply_permissions_update(state, next.clone()).await?;
    send_permissions_updated(ws_sink, &next).await?;

    Ok(())
}

async fn apply_permissions_update(state: &AppState, next: PermissionsConfig) -> anyhow::Result<()> {
    let engine = PermissionEngine::from_config(&next)?;
    state.config_store.save_permissions(&next)?;

    {
        let mut permissions = state.permissions_config.write().await;
        *permissions = next.clone();
    }
    {
        let mut permission_engine = state.permission_engine.write().await;
        *permission_engine = engine;
    }

    Ok(())
}

fn sanitize_rules(raw: Option<&Vec<String>>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };

    raw.iter()
        .map(|rule| rule.trim())
        .filter(|rule| !rule.is_empty())
        .map(ToString::to_string)
        .collect()
}

async fn handle_thread_command(
    state: &AppState,
    client_msg: &ClientMessage,
    current_session_id: &mut String,
    history: &mut Vec<ChatMessage>,
) -> anyhow::Result<Option<serde_json::Value>> {
    match client_msg.msg_type.as_str() {
        "thread_list" => {
            let sessions = state.sessions.lock().await;
            Ok(Some(serde_json::json!({
                "type": "thread_list",
                "current_session_id": sessions.current_session_id(),
                "sessions": sessions.list_sessions(),
            })))
        }
        "thread_create" => {
            let mut sessions = state.sessions.lock().await;
            let created = sessions.create_session(client_msg.display_name.as_deref())?;
            *current_session_id = created.id.clone();
            history.clear();
            Ok(Some(serde_json::json!({
                "type": "thread_created",
                "session": created,
                "current_session_id": sessions.current_session_id(),
                "sessions": sessions.list_sessions(),
                "history": Vec::<ChatMessage>::new(),
            })))
        }
        "thread_rename" => {
            let sid = required_field(client_msg.session_id.as_deref(), "session_id")?;
            let display_name = required_field(client_msg.display_name.as_deref(), "display_name")?;
            let mut sessions = state.sessions.lock().await;
            let record = sessions.rename_session(sid, display_name)?;
            Ok(Some(serde_json::json!({
                "type": "thread_renamed",
                "session": record,
                "current_session_id": sessions.current_session_id(),
                "sessions": sessions.list_sessions(),
            })))
        }
        "thread_switch" => {
            let sid = required_field(client_msg.session_id.as_deref(), "session_id")?;
            let mut sessions = state.sessions.lock().await;
            let (_record, loaded_history) = sessions.switch_session(sid)?;
            *current_session_id = sid.to_string();
            *history = loaded_history.clone();
            Ok(Some(serde_json::json!({
                "type": "thread_switched",
                "session_id": sid,
                "history": loaded_history,
                "current_session_id": sessions.current_session_id(),
                "sessions": sessions.list_sessions(),
            })))
        }
        "thread_clear" => {
            let mut sessions = state.sessions.lock().await;
            let (record, loaded_history) = sessions.clear_current_session()?;
            *current_session_id = record.id.clone();
            *history = loaded_history.clone();
            Ok(Some(serde_json::json!({
                "type": "thread_cleared",
                "session_id": record.id,
                "history": loaded_history,
                "current_session_id": sessions.current_session_id(),
                "sessions": sessions.list_sessions(),
            })))
        }
        "thread_delete" => {
            let sid = required_field(client_msg.session_id.as_deref(), "session_id")?;
            let mut sessions = state.sessions.lock().await;
            let deleted = sessions.delete_session(sid)?;
            let loaded_history = sessions.load_history(&deleted.current_session_id)?;
            *current_session_id = deleted.current_session_id.clone();
            *history = loaded_history.clone();
            Ok(Some(serde_json::json!({
                "type": "thread_deleted",
                "deleted_session_id": deleted.deleted_session_id,
                "current_session_id": deleted.current_session_id,
                "sessions": sessions.list_sessions(),
                "history": loaded_history,
            })))
        }
        _ => Ok(None),
    }
}

fn required_field<'a>(value: Option<&'a str>, field_name: &str) -> anyhow::Result<&'a str> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required field '{}'", field_name))
}

fn is_thread_mutating_command(msg_type: &str) -> bool {
    matches!(
        msg_type,
        "thread_create" | "thread_rename" | "thread_switch" | "thread_clear" | "thread_delete"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use uuid::Uuid;

    use crate::agent::Agent;
    use crate::config::{PermissionsConfig, ToolPermissionsConfig};
    use crate::permissions::PermissionEngine;
    use crate::prompt::{PromptLimits, PromptManager};
    use crate::providers::Provider;
    use crate::session::SessionManager;
    use crate::tools::ToolRegistry;

    struct DummyProvider;

    #[async_trait::async_trait]
    impl Provider for DummyProvider {
        fn supports_native_tools(&self) -> bool {
            false
        }

        async fn chat(
            &self,
            _messages: &[crate::providers::ChatMessage],
            _tools: Option<&[crate::providers::ToolSpec]>,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<crate::providers::ChatResponse> {
            Ok(crate::providers::ChatResponse {
                text: Some("ok".to_string()),
                tool_calls: Vec::new(),
                usage: None,
            })
        }
    }

    fn temp_workspace(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rikabot-ws-test-{}-{}", name, Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    fn test_state(workspace: &PathBuf) -> AppState {
        let provider: Box<dyn Provider> = Box::new(DummyProvider);
        let agent = Arc::new(Agent::new(
            provider,
            ToolRegistry::new(),
            "model".to_string(),
            0.1,
        ));
        let sessions = Arc::new(tokio::sync::Mutex::new(
            SessionManager::new(workspace).expect("create sessions"),
        ));
        let prompt_manager = Arc::new(
            PromptManager::new(
                workspace,
                "system".to_string(),
                false,
                PromptLimits {
                    bootstrap_max_chars: 20_000,
                    bootstrap_total_max_chars: 150_000,
                },
            )
            .expect("create prompt manager"),
        );

        AppState {
            agent,
            sessions,
            prompt_manager,
            runs: Arc::new(tokio::sync::Mutex::new(
                crate::gateway::RunManager::default(),
            )),
            permissions_config: Arc::new(tokio::sync::RwLock::new(PermissionsConfig {
                enabled: false,
                tools: ToolPermissionsConfig::default(),
            })),
            permission_engine: Arc::new(tokio::sync::RwLock::new(
                PermissionEngine::disabled_allow_all(),
            )),
            config_store: Arc::new(crate::config_store::ConfigStore::new(
                workspace.join("config.toml"),
            )),
            mcp_runtime: Arc::new(crate::mcp_runtime::McpRuntime::new(false, &[])),
        }
    }

    #[tokio::test]
    async fn thread_commands_create_rename_switch_clear_delete_update_state() {
        let workspace = temp_workspace("thread-commands");
        let state = test_state(&workspace);

        let (mut current, mut history) = hydrate_current_thread(&state).await.expect("hydrate");

        let created = handle_thread_command(
            &state,
            &ClientMessage {
                msg_type: "thread_create".to_string(),
                content: None,
                session_id: None,
                display_name: Some("alpha".to_string()),
                enabled: None,
                allow: None,
                deny: None,
                request_id: None,
                decision: None,
                allow_rule: None,
            },
            &mut current,
            &mut history,
        )
        .await
        .expect("create")
        .expect("event");
        assert_eq!(created["type"], "thread_created");
        let created_id = created["session"]["id"].as_str().expect("created id");

        let renamed = handle_thread_command(
            &state,
            &ClientMessage {
                msg_type: "thread_rename".to_string(),
                content: None,
                session_id: Some(created_id.to_string()),
                display_name: Some("renamed".to_string()),
                enabled: None,
                allow: None,
                deny: None,
                request_id: None,
                decision: None,
                allow_rule: None,
            },
            &mut current,
            &mut history,
        )
        .await
        .expect("rename")
        .expect("event");
        assert_eq!(renamed["type"], "thread_renamed");
        assert_eq!(renamed["session"]["display_name"], "renamed");

        let switched = handle_thread_command(
            &state,
            &ClientMessage {
                msg_type: "thread_switch".to_string(),
                content: None,
                session_id: Some(created_id.to_string()),
                display_name: None,
                enabled: None,
                allow: None,
                deny: None,
                request_id: None,
                decision: None,
                allow_rule: None,
            },
            &mut current,
            &mut history,
        )
        .await
        .expect("switch")
        .expect("event");
        assert_eq!(switched["type"], "thread_switched");
        assert_eq!(
            switched["session_id"].as_str().expect("switched id"),
            created_id
        );

        let cleared = handle_thread_command(
            &state,
            &ClientMessage {
                msg_type: "thread_clear".to_string(),
                content: None,
                session_id: None,
                display_name: None,
                enabled: None,
                allow: None,
                deny: None,
                request_id: None,
                decision: None,
                allow_rule: None,
            },
            &mut current,
            &mut history,
        )
        .await
        .expect("clear")
        .expect("event");
        assert_eq!(cleared["type"], "thread_cleared");
        assert_eq!(cleared["history"].as_array().expect("history").len(), 0);

        let deleted = handle_thread_command(
            &state,
            &ClientMessage {
                msg_type: "thread_delete".to_string(),
                content: None,
                session_id: Some(created_id.to_string()),
                display_name: None,
                enabled: None,
                allow: None,
                deny: None,
                request_id: None,
                decision: None,
                allow_rule: None,
            },
            &mut current,
            &mut history,
        )
        .await
        .expect("delete")
        .expect("event");
        assert_eq!(deleted["type"], "thread_deleted");
        assert_eq!(deleted["deleted_session_id"], created_id);
        assert_ne!(deleted["current_session_id"], created_id);
    }

    #[tokio::test]
    async fn thread_commands_validate_required_fields() {
        let workspace = temp_workspace("validation");
        let state = test_state(&workspace);
        let (mut current, mut history) = hydrate_current_thread(&state).await.expect("hydrate");

        let err = handle_thread_command(
            &state,
            &ClientMessage {
                msg_type: "thread_rename".to_string(),
                content: None,
                session_id: None,
                display_name: Some("x".to_string()),
                enabled: None,
                allow: None,
                deny: None,
                request_id: None,
                decision: None,
                allow_rule: None,
            },
            &mut current,
            &mut history,
        )
        .await
        .expect_err("should reject missing session_id");
        assert!(err.to_string().contains("session_id"));
    }

    #[test]
    fn thread_mutation_command_detection() {
        assert!(is_thread_mutating_command("thread_create"));
        assert!(is_thread_mutating_command("thread_rename"));
        assert!(is_thread_mutating_command("thread_switch"));
        assert!(is_thread_mutating_command("thread_clear"));
        assert!(is_thread_mutating_command("thread_delete"));

        assert!(!is_thread_mutating_command("thread_list"));
        assert!(!is_thread_mutating_command("kill_switch"));
        assert!(!is_thread_mutating_command("message"));
    }
}
