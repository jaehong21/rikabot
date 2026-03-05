use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot};
use uuid::Uuid;

use crate::agent::{AgentEvent, ToolApprovalDecision, ToolApprovalDecisionKind};
use crate::config::PermissionsConfig;
use crate::gateway::{ActiveRunState, AppState, QueuedInput, RunManager};
use crate::mcp_runtime::McpStatusSnapshot;
use crate::permissions::PermissionEngine;
use crate::prompt::SessionPromptContext;
use crate::providers::ChatMessage;

const MAX_QUEUED_MESSAGES_PER_SESSION: usize = 5;

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
    request_id: Option<String>,
    #[serde(default)]
    decision: Option<ToolApprovalDecisionKind>,
    #[serde(default)]
    allow_rule: Option<String>,
    #[serde(default)]
    queue_item_id: Option<String>,
}

struct RunOutcome {
    result: anyhow::Result<()>,
}

struct SessionRunInput {
    history: Vec<ChatMessage>,
    session_display_name: String,
}

enum SubmitOutcome {
    Reserved,
    Queued,
}

pub async fn spawn_session_change_watcher(state: AppState) {
    let sessions_dir = {
        let sessions = state.sessions.lock().await;
        sessions.sessions_dir_path().to_path_buf()
    };

    tokio::spawn(async move {
        if let Err(err) = run_session_change_watcher(state, sessions_dir).await {
            tracing::warn!("session change watcher stopped: {}", err);
        }
    });
}

/// Axum handler that upgrades an HTTP request to a WebSocket connection.
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle a single WebSocket connection.
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_sink, mut ws_stream) = socket.split();
    let (run_signal_tx, mut run_signal_rx) = mpsc::unbounded_channel::<Value>();
    let mut thread_event_rx = state.thread_events.subscribe();
    let mut mcp_status_rx = state.mcp_runtime.subscribe();

    let mut current_session_id = match hydrate_current_thread(&state).await {
        Ok((sid, _initial_history)) => {
            if send_mcp_status(&mut ws_sink, state.mcp_runtime.snapshot())
                .await
                .is_err()
            {
                return;
            }
            sid
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

    if let Err(err) = attach_to_runtime(&state, run_signal_tx.clone(), &mut ws_sink).await {
        let _ = send_error(
            &mut ws_sink,
            &format!("Failed to attach runtime state: {}", err),
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
                        let content = match client_msg.content {
                            Some(c) if !c.trim().is_empty() => c,
                            _ => continue,
                        };

                        let target_session_id = client_msg
                            .session_id
                            .as_deref()
                            .map(str::trim)
                            .filter(|sid| !sid.is_empty())
                            .unwrap_or(current_session_id.as_str())
                            .to_string();
                        current_session_id = target_session_id.clone();

                        let submit_result = submit_or_queue_message(
                            &state,
                            &target_session_id,
                            content,
                        )
                        .await;
                        if let Err(err) = submit_result {
                            let _ = send_error(
                                &mut ws_sink,
                                &format!("Failed to submit message: {}", err),
                            )
                            .await;
                        }
                    }
                    "kill_switch" => {
                        let requested_session = client_msg
                            .session_id
                            .as_deref()
                            .map(str::trim)
                            .filter(|sid| !sid.is_empty())
                            .unwrap_or(current_session_id.as_str());
                        if !stop_active_run(&state, "user_cancelled", Some(requested_session)).await {
                            let _ = send_stopped(&mut ws_sink, "no_active_run", Some(requested_session)).await;
                        }
                    }
                    "queue_cancel" => {
                        let requested_session = client_msg
                            .session_id
                            .as_deref()
                            .map(str::trim)
                            .filter(|sid| !sid.is_empty())
                            .unwrap_or(current_session_id.as_str());

                        let queue_item_id = client_msg
                            .queue_item_id
                            .as_deref()
                            .map(str::trim)
                            .filter(|id| !id.is_empty());

                        let changed = cancel_queued_inputs(&state, requested_session, queue_item_id).await;
                        if !changed {
                            let _ = send_error(
                                &mut ws_sink,
                                "No queued message matched the requested cancellation.",
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
                        let _ = send_error(
                            &mut ws_sink,
                            &format!("Unsupported WebSocket message type: {}", client_msg.msg_type),
                        )
                        .await;
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
            thread_event = thread_event_rx.recv() => {
                match thread_event {
                    Ok(payload) => {
                        if ws_sink
                            .send(Message::text(payload.to_string()))
                            .await
                        .is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        if send_thread_list(&mut ws_sink, &state).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
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

async fn run_session_change_watcher(state: AppState, sessions_dir: PathBuf) -> anyhow::Result<()> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(
        move |result| {
            let _ = event_tx.send(result);
        },
        notify::Config::default(),
    )?;

    watcher.watch(&sessions_dir, RecursiveMode::NonRecursive)?;

    while let Some(event_result) = event_rx.recv().await {
        let event = match event_result {
            Ok(event) => event,
            Err(err) => {
                tracing::warn!("session watcher event error: {}", err);
                continue;
            }
        };

        if !is_relevant_session_event(&event) {
            continue;
        }

        // Coalesce bursty rename/write notifications into one reload + broadcast.
        tokio::time::sleep(Duration::from_millis(75)).await;
        while let Ok(next_result) = event_rx.try_recv() {
            if let Err(err) = next_result {
                tracing::warn!("session watcher event error: {}", err);
            }
        }

        if let Err(err) = broadcast_reloaded_thread_list(&state).await {
            tracing::warn!("failed to broadcast session update: {}", err);
        }
    }

    Ok(())
}

fn is_relevant_session_event(event: &Event) -> bool {
    if !matches!(
        event.kind,
        EventKind::Any
            | EventKind::Create(_)
            | EventKind::Modify(_)
            | EventKind::Remove(_)
            | EventKind::Other
    ) {
        return false;
    }

    if event.paths.is_empty() {
        return true;
    }

    event
        .paths
        .iter()
        .any(|path| is_session_file_path(path.as_path()))
}

fn is_session_file_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "sessions.json" || name.ends_with(".jsonl"))
}

async fn submit_or_queue_message(
    state: &AppState,
    session_id: &str,
    content: String,
) -> anyhow::Result<()> {
    let submit_outcome = reserve_or_queue_submission(state, session_id, content.clone()).await?;
    if matches!(submit_outcome, SubmitOutcome::Queued) {
        return Ok(());
    }

    let run_input = match load_session_run_input(state, session_id).await {
        Ok(input) => input,
        Err(err) => {
            release_starting_slot(state, session_id).await;
            return Err(err);
        }
    };

    if let Err(err) = spawn_reserved_run(
        state.clone(),
        session_id.to_string(),
        run_input.history,
        run_input.session_display_name,
        content,
    )
    .await
    {
        release_starting_slot(state, session_id).await;
        return Err(err);
    }

    Ok(())
}

async fn reserve_or_queue_submission(
    state: &AppState,
    session_id: &str,
    content: String,
) -> anyhow::Result<SubmitOutcome> {
    let mut runs = state.runs.lock().await;
    let session_busy =
        runs.active.contains_key(session_id) || runs.starting_sessions.contains(session_id);
    let at_capacity =
        runs.active.len() + runs.starting_sessions.len() >= runs.max_concurrent_sessions;

    if session_busy || at_capacity {
        let queue = runs.queues.entry(session_id.to_string()).or_default();
        if queue.len() >= MAX_QUEUED_MESSAGES_PER_SESSION {
            anyhow::bail!(
                "Queue is full for session {} (max {} queued user messages).",
                session_id,
                MAX_QUEUED_MESSAGES_PER_SESSION
            );
        }

        queue.push_back(QueuedInput {
            id: Uuid::new_v4().to_string(),
            content,
        });

        let payload = build_queue_updated_payload(session_id, queue);
        broadcast_payload_locked(&mut runs, payload);
        return Ok(SubmitOutcome::Queued);
    }

    runs.starting_sessions.insert(session_id.to_string());
    Ok(SubmitOutcome::Reserved)
}

async fn spawn_reserved_run(
    state: AppState,
    session_id: String,
    history: Vec<ChatMessage>,
    session_display_name: String,
    content: String,
) -> anyhow::Result<()> {
    let system_prompt =
        state
            .prompt_manager
            .build_prompt_with_session(Some(&SessionPromptContext {
                session_id: session_id.clone(),
                session_display_name: session_display_name.clone(),
            }))?;

    append_session_messages(&state, &session_id, &[ChatMessage::user(&content)]).await?;

    let run_id = {
        let mut runs_guard = state.runs.lock().await;
        if !runs_guard.starting_sessions.contains(&session_id) {
            anyhow::bail!("run slot reservation missing for session {}", session_id);
        }
        let run_id = runs_guard.next_run_id;
        runs_guard.next_run_id = runs_guard.next_run_id.saturating_add(1);
        run_id
    };

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (approval_tx, approval_rx) = mpsc::unbounded_channel::<ToolApprovalDecision>();
    let (outcome_tx, outcome_rx) = oneshot::channel::<RunOutcome>();
    let agent = state.agent.clone();
    let run_store = state.runs.clone();
    let state_for_task = state.clone();
    let mut history_clone = history;
    let session_id_owned = session_id.clone();
    let content_for_user_event = content.clone();

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
        let _ = outcome_tx.send(RunOutcome { result });
    });

    let session_id_for_task = session_id_owned.clone();
    let event_task = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if let AgentEvent::ToolApprovalRequired { request_id, .. } = &event {
                register_pending_approval(&run_store, &session_id_for_task, run_id, request_id)
                    .await;
            }
            if let AgentEvent::ToolCallResult {
                approval_request_id: Some(request_id),
                awaiting_approval: false,
                ..
            } = &event
            {
                clear_pending_approval(&run_store, &session_id_for_task, run_id, request_id).await;
            }

            if let Err(err) =
                persist_agent_event(&state_for_task, &session_id_for_task, &event).await
            {
                let payload = serde_json::json!({
                    "type": "error",
                    "message": format!("Failed to persist session message: {}", err),
                });
                broadcast_run_payload(&run_store, &session_id_for_task, run_id, payload).await;
            }

            let payload = match serde_json::to_value(&event) {
                Ok(value) => value,
                Err(_) => continue,
            };
            broadcast_run_payload(&run_store, &session_id_for_task, run_id, payload).await;
        }

        let outcome_ok = matches!(outcome_rx.await, Ok(RunOutcome { result: Ok(()) }));
        clear_active_run(&run_store, &session_id_for_task, run_id).await;
        if outcome_ok {
            let state_for_scheduler = state_for_task.clone();
            let handle = tokio::runtime::Handle::current();
            // Queue scheduling uses a non-Send future path; run it from a blocking lane.
            tokio::task::spawn_blocking(move || {
                handle.block_on(maybe_start_queued_runs(state_for_scheduler));
            });
        }
        let _ = state_for_task;
    });

    {
        let mut runs_guard = state.runs.lock().await;
        if runs_guard.active.contains_key(&session_id) {
            runs_guard.starting_sessions.remove(&session_id);
            agent_task.abort();
            event_task.abort();
            anyhow::bail!("active run already exists for session {}", session_id);
        }

        runs_guard.starting_sessions.remove(&session_id);

        let user_payload = with_run_scope(
            serde_json::json!({
                "type": "user_message",
                "content": content_for_user_event,
            }),
            &session_id,
            run_id,
        );

        let active = ActiveRunState {
            run_id,
            events: vec![user_payload.clone()],
            approval_tx,
            pending_approval_ids: std::collections::HashSet::new(),
            agent_task,
            event_task,
        };
        runs_guard.active.insert(session_id.clone(), active);
        broadcast_payload_locked(&mut runs_guard, user_payload);
    }

    Ok(())
}

async fn persist_agent_event(
    state: &AppState,
    session_id: &str,
    event: &AgentEvent,
) -> anyhow::Result<()> {
    match event {
        AgentEvent::ToolCallStart {
            call_id,
            name,
            args,
        } => {
            let arguments = if let Some(raw) = args.as_str() {
                raw.to_string()
            } else {
                args.to_string()
            };
            let assistant_content = serde_json::json!({
                "tool_calls": [{
                    "id": call_id,
                    "name": name,
                    "arguments": arguments,
                }],
                "content": "",
            })
            .to_string();
            append_session_messages(
                state,
                session_id,
                &[ChatMessage::assistant(&assistant_content)],
            )
            .await
        }
        AgentEvent::ToolCallResult {
            call_id,
            output,
            status,
            awaiting_approval,
            ..
        } => {
            if *awaiting_approval {
                return Ok(());
            }

            let status_value = serde_json::to_value(status)
                .ok()
                .and_then(|raw| raw.as_str().map(ToString::to_string))
                .unwrap_or_else(|| "failed".to_string());

            let tool_msg_content = serde_json::json!({
                "tool_call_id": call_id,
                "content": output,
                "status": status_value,
            })
            .to_string();
            append_session_messages(state, session_id, &[ChatMessage::tool(&tool_msg_content)])
                .await
        }
        AgentEvent::Done { full_response, .. } => {
            append_session_messages(state, session_id, &[ChatMessage::assistant(full_response)])
                .await
        }
        AgentEvent::Error { message } => {
            let note = format!("Error: {}", message);
            append_session_messages(state, session_id, &[ChatMessage::assistant(&note)]).await
        }
        AgentEvent::Chunk { .. } | AgentEvent::ToolApprovalRequired { .. } => Ok(()),
    }
}

async fn append_session_messages(
    state: &AppState,
    session_id: &str,
    messages: &[ChatMessage],
) -> anyhow::Result<()> {
    if messages.is_empty() {
        return Ok(());
    }

    let mut sessions = state.sessions.lock().await;
    sessions.append_messages(session_id, messages)
}

async fn maybe_start_queued_runs(state: AppState) {
    loop {
        let next = {
            let mut runs = state.runs.lock().await;
            if runs.active.len() + runs.starting_sessions.len() >= runs.max_concurrent_sessions {
                None
            } else {
                let mut candidate_session_id: Option<String> = None;
                for (session_id, queue) in &runs.queues {
                    if queue.is_empty()
                        || runs.active.contains_key(session_id)
                        || runs.starting_sessions.contains(session_id)
                    {
                        continue;
                    }
                    candidate_session_id = Some(session_id.clone());
                    break;
                }

                let Some(session_id) = candidate_session_id else {
                    return;
                };

                let content = {
                    let queue = runs
                        .queues
                        .get_mut(&session_id)
                        .expect("queue must exist for selected session");
                    let queued = queue
                        .pop_front()
                        .expect("queue must contain at least one item");
                    let content = queued.content;
                    let queue_payload = build_queue_updated_payload(&session_id, queue);
                    broadcast_payload_locked(&mut runs, queue_payload);
                    content
                };

                let remove_queue = runs
                    .queues
                    .get(&session_id)
                    .is_some_and(|queue| queue.is_empty());
                if remove_queue {
                    runs.queues.remove(&session_id);
                }

                runs.starting_sessions.insert(session_id.clone());
                Some((session_id, content))
            }
        };

        let Some((session_id, content)) = next else {
            return;
        };

        let run_input = match load_session_run_input(&state, &session_id).await {
            Ok(input) => input,
            Err(err) => {
                release_starting_slot(&state, &session_id).await;
                broadcast_session_error(
                    &state,
                    &session_id,
                    &format!("Failed to load queued session history: {}", err),
                )
                .await;
                continue;
            }
        };

        if let Err(err) = spawn_reserved_run(
            state.clone(),
            session_id.clone(),
            run_input.history,
            run_input.session_display_name,
            content,
        )
        .await
        {
            release_starting_slot(&state, &session_id).await;
            broadcast_session_error(
                &state,
                &session_id,
                &format!("Failed to start queued run: {}", err),
            )
            .await;
        }
    }
}

async fn release_starting_slot(state: &AppState, session_id: &str) {
    let mut runs = state.runs.lock().await;
    runs.starting_sessions.remove(session_id);
}

async fn cancel_queued_inputs(
    state: &AppState,
    session_id: &str,
    queue_item_id: Option<&str>,
) -> bool {
    let mut runs = state.runs.lock().await;
    let Some(queue) = runs.queues.get_mut(session_id) else {
        return false;
    };

    let changed = if let Some(item_id) = queue_item_id {
        let maybe_idx = queue.iter().position(|item| item.id == item_id);
        if let Some(idx) = maybe_idx {
            queue.remove(idx);
            true
        } else {
            false
        }
    } else if queue.is_empty() {
        false
    } else {
        queue.clear();
        true
    };

    if !changed {
        return false;
    }

    let payload = build_queue_updated_payload(session_id, queue);
    let remove_queue = queue.is_empty();
    if remove_queue {
        runs.queues.remove(session_id);
    }
    broadcast_payload_locked(&mut runs, payload);
    true
}

fn build_queue_updated_payload(
    session_id: &str,
    queue: &std::collections::VecDeque<QueuedInput>,
) -> Value {
    let items: Vec<QueuedInput> = queue.iter().cloned().collect();
    serde_json::json!({
        "type": "queue_updated",
        "session_id": session_id,
        "items": items,
    })
}

fn broadcast_payload_locked(runs: &mut RunManager, payload: Value) {
    runs.subscribers
        .retain(|sub| sub.send(payload.clone()).is_ok());
}

fn with_run_scope(payload: Value, session_id: &str, run_id: u64) -> Value {
    let mut payload_obj = match payload {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    payload_obj.insert(
        "session_id".to_string(),
        Value::String(session_id.to_string()),
    );
    payload_obj.insert(
        "run_id".to_string(),
        Value::Number(serde_json::Number::from(run_id)),
    );
    Value::Object(payload_obj)
}

async fn broadcast_run_payload(
    runs: &std::sync::Arc<tokio::sync::Mutex<RunManager>>,
    session_id: &str,
    run_id: u64,
    payload: Value,
) {
    let mut runs = runs.lock().await;
    let scoped_payload = with_run_scope(payload, session_id, run_id);

    if let Some(active) = runs.active.get_mut(session_id) {
        if active.run_id == run_id {
            active.events.push(scoped_payload.clone());
        }
    }

    broadcast_payload_locked(&mut runs, scoped_payload);
}

async fn broadcast_session_error(state: &AppState, session_id: &str, message: &str) {
    let payload = serde_json::json!({
        "type": "error",
        "session_id": session_id,
        "message": message,
    });

    let mut runs = state.runs.lock().await;
    broadcast_payload_locked(&mut runs, payload);
}

async fn register_pending_approval(
    runs: &std::sync::Arc<tokio::sync::Mutex<RunManager>>,
    session_id: &str,
    run_id: u64,
    request_id: &str,
) {
    let mut runs = runs.lock().await;
    let Some(active) = runs.active.get_mut(session_id) else {
        return;
    };
    if active.run_id != run_id {
        return;
    }
    active.pending_approval_ids.insert(request_id.to_string());
}

async fn clear_pending_approval(
    runs: &std::sync::Arc<tokio::sync::Mutex<RunManager>>,
    session_id: &str,
    run_id: u64,
    request_id: &str,
) {
    let mut runs = runs.lock().await;
    let Some(active) = runs.active.get_mut(session_id) else {
        return;
    };
    if active.run_id != run_id {
        return;
    }
    active.pending_approval_ids.remove(request_id);
}

async fn clear_active_run(
    runs: &std::sync::Arc<tokio::sync::Mutex<RunManager>>,
    session_id: &str,
    run_id: u64,
) {
    let mut runs = runs.lock().await;
    let should_remove = runs
        .active
        .get(session_id)
        .is_some_and(|active| active.run_id == run_id);
    if should_remove {
        runs.active.remove(session_id);
    }
}

async fn load_session_run_input(
    state: &AppState,
    session_id: &str,
) -> anyhow::Result<SessionRunInput> {
    let mut sessions = state.sessions.lock().await;
    sessions.reload_from_disk()?;
    let history = sessions.load_history(session_id)?;
    let session_display_name = sessions
        .get_session(session_id)
        .map(|record| record.display_name)
        .unwrap_or_else(|| session_id.to_string());
    Ok(SessionRunInput {
        history,
        session_display_name,
    })
}

async fn attach_to_runtime(
    state: &AppState,
    run_signal_tx: mpsc::UnboundedSender<Value>,
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    let (mut replay, queue_payloads) = {
        let mut runs = state.runs.lock().await;
        runs.subscribers.push(run_signal_tx);

        let mut replay: Vec<Value> = runs
            .active
            .values()
            .flat_map(|active| active.events.clone())
            .collect();
        replay.sort_by_key(|payload| payload.get("run_id").and_then(Value::as_u64).unwrap_or(0));

        let queue_payloads: Vec<Value> = runs
            .queues
            .iter()
            .map(|(session_id, queue)| build_queue_updated_payload(session_id, queue))
            .collect();

        (replay, queue_payloads)
    };

    for payload in replay.drain(..) {
        ws_sink.send(Message::text(payload.to_string())).await?;
    }

    for payload in queue_payloads {
        ws_sink.send(Message::text(payload.to_string())).await?;
    }

    Ok(())
}

async fn stop_active_run(state: &AppState, reason: &str, session_id: Option<&str>) -> bool {
    let requested_session_id = session_id.map(str::to_string);
    let Some(requested_session_id) = requested_session_id else {
        return false;
    };

    let run = {
        let mut runs = state.runs.lock().await;
        let queue_cleared = if let Some(queue) = runs.queues.get_mut(&requested_session_id) {
            let had_items = !queue.is_empty();
            if had_items {
                queue.clear();
                let payload = build_queue_updated_payload(&requested_session_id, queue);
                broadcast_payload_locked(&mut runs, payload);
            }
            had_items
        } else {
            false
        };

        if queue_cleared {
            runs.queues.remove(&requested_session_id);
        }

        runs.active.remove(&requested_session_id)
    };

    let Some(run) = run else {
        return false;
    };

    let stopped_payload = with_run_scope(
        serde_json::json!({
            "type": "stopped",
            "reason": reason,
        }),
        &requested_session_id,
        run.run_id,
    );
    {
        let mut runs = state.runs.lock().await;
        broadcast_payload_locked(&mut runs, stopped_payload);
    }

    let stop_note = match reason {
        "user_cancelled" => "Stopped by user.".to_string(),
        other => format!("Stopped: {}", other),
    };
    if let Err(err) = append_session_messages(
        state,
        &requested_session_id,
        &[ChatMessage::assistant(&stop_note)],
    )
    .await
    {
        broadcast_session_error(
            state,
            &requested_session_id,
            &format!("Failed to persist stop message: {}", err),
        )
        .await;
    }

    run.agent_task.abort();
    run.event_task.abort();
    let _ = run.agent_task.await;
    let _ = run.event_task.await;

    maybe_start_queued_runs(state.clone()).await;
    true
}

async fn hydrate_current_thread(state: &AppState) -> anyhow::Result<(String, Vec<ChatMessage>)> {
    let mut sessions = state.sessions.lock().await;
    sessions.reload_from_disk()?;
    let sid = sessions.current_session_id().to_string();
    let history = sessions.load_history(&sid)?;
    Ok((sid, history))
}

async fn build_thread_list_payload(
    state: &AppState,
    reload_from_disk: bool,
) -> anyhow::Result<Value> {
    build_thread_list_payload_from_sessions(&state.sessions, reload_from_disk).await
}

async fn build_thread_list_payload_from_sessions(
    sessions: &std::sync::Arc<tokio::sync::Mutex<crate::session::SessionManager>>,
    reload_from_disk: bool,
) -> anyhow::Result<Value> {
    let payload = {
        let mut sessions = sessions.lock().await;
        if reload_from_disk {
            sessions.reload_from_disk()?;
        }
        serde_json::json!({
            "type": "thread_list",
            "current_session_id": sessions.current_session_id(),
            "sessions": sessions.list_sessions(),
        })
    };
    Ok(payload)
}

async fn broadcast_reloaded_thread_list(state: &AppState) -> anyhow::Result<()> {
    let payload = build_thread_list_payload(state, true).await?;
    let _ = state.thread_events.send(payload);
    Ok(())
}

async fn send_thread_list(
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
) -> anyhow::Result<()> {
    let payload = build_thread_list_payload(state, true).await?;
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
        let mut found: Option<mpsc::UnboundedSender<ToolApprovalDecision>> = None;

        for active in runs.active.values_mut() {
            if active.pending_approval_ids.remove(&request_id) {
                found = Some(active.approval_tx.clone());
                break;
            }
        }

        found.ok_or_else(|| {
            anyhow::anyhow!(
                "unknown or expired tool approval request_id '{}'",
                request_id
            )
        })?
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

fn required_field<'a>(value: Option<&'a str>, field_name: &str) -> anyhow::Result<&'a str> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required field '{}'", field_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use uuid::Uuid;

    use crate::agent::Agent;
    use crate::config::{PermissionsConfig, ToolPermissionsConfig};
    use crate::permissions::PermissionEngine;
    use crate::prompt::{PromptLimits, PromptManager};
    use crate::providers::{ChatMessage, ChatResponse, Provider, ToolSpec};
    use crate::session::SessionManager;
    use crate::tools::ToolCallStatus;
    use crate::tools::ToolRegistry;

    struct WsTestProvider;

    #[async_trait::async_trait]
    impl Provider for WsTestProvider {
        fn supports_native_tools(&self) -> bool {
            true
        }

        async fn chat(
            &self,
            messages: &[ChatMessage],
            _tools: Option<&[ToolSpec]>,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<ChatResponse> {
            let prompt = messages
                .iter()
                .rev()
                .find(|message| message.role == "user")
                .map(|message| message.content.trim().to_string())
                .unwrap_or_default();

            if prompt.starts_with("slow-ok:") {
                tokio::time::sleep(Duration::from_millis(200)).await;
            } else if prompt.starts_with("slow-fail:") {
                tokio::time::sleep(Duration::from_millis(200)).await;
                anyhow::bail!("mock failure for {}", prompt);
            }

            Ok(ChatResponse {
                text: Some(format!("mock-backend: {}", prompt)),
                tool_calls: Vec::new(),
                usage: None,
            })
        }
    }

    fn temp_workspace(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("rika-ws-test-{}-{}", name, Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temp workspace");
        path
    }

    fn test_state(workspace: &PathBuf, max_concurrent_sessions: usize) -> AppState {
        let provider: Box<dyn Provider> = Box::new(WsTestProvider);
        let agent = Arc::new(Agent::new(
            provider,
            ToolRegistry::new(),
            "test-model".to_string(),
            0.1,
        ));
        let sessions = Arc::new(tokio::sync::Mutex::new(
            SessionManager::new(workspace).expect("create sessions"),
        ));
        let prompt_manager = Arc::new(
            PromptManager::new(
                workspace,
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
            runs: Arc::new(tokio::sync::Mutex::new(RunManager::new(
                max_concurrent_sessions,
            ))),
            thread_events: tokio::sync::broadcast::channel(64).0,
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

    async fn current_session_id(state: &AppState) -> String {
        let mut sessions = state.sessions.lock().await;
        sessions.reload_from_disk().expect("reload sessions");
        sessions.current_session_id().to_string()
    }

    async fn load_session_history(state: &AppState, session_id: &str) -> Vec<ChatMessage> {
        let mut sessions = state.sessions.lock().await;
        sessions.reload_from_disk().expect("reload sessions");
        sessions
            .load_history(session_id)
            .expect("load session history")
    }

    async fn create_session(state: &AppState, display_name: &str) -> String {
        let mut sessions = state.sessions.lock().await;
        sessions.reload_from_disk().expect("reload sessions");
        sessions
            .create_session(Some(display_name))
            .expect("create session")
            .id
    }

    #[tokio::test]
    async fn reserve_or_queue_submission_enforces_queue_cap_per_session() {
        let workspace = temp_workspace("queue-cap");
        let state = test_state(&workspace, 8);
        let session_id = current_session_id(&state).await;

        {
            let mut runs = state.runs.lock().await;
            runs.starting_sessions.insert(session_id.clone());
            let queue = runs.queues.entry(session_id.clone()).or_default();
            for idx in 0..MAX_QUEUED_MESSAGES_PER_SESSION {
                queue.push_back(QueuedInput {
                    id: format!("q-{}", idx),
                    content: format!("queued-{}", idx),
                });
            }
        }

        let err =
            match reserve_or_queue_submission(&state, &session_id, "overflow".to_string()).await {
                Ok(_) => panic!("expected queue cap overflow to fail"),
                Err(err) => err.to_string(),
            };
        assert!(err.contains("Queue is full"));
    }

    #[tokio::test]
    async fn cancel_queued_inputs_removes_target_and_clear_all() {
        let workspace = temp_workspace("queue-cancel");
        let state = test_state(&workspace, 8);
        let session_id = current_session_id(&state).await;
        let (subscriber_tx, mut subscriber_rx) = mpsc::unbounded_channel::<Value>();

        {
            let mut runs = state.runs.lock().await;
            runs.subscribers.push(subscriber_tx);
            let queue = runs.queues.entry(session_id.clone()).or_default();
            queue.push_back(QueuedInput {
                id: "keep".to_string(),
                content: "keep".to_string(),
            });
            queue.push_back(QueuedInput {
                id: "drop".to_string(),
                content: "drop".to_string(),
            });
        }

        let removed = cancel_queued_inputs(&state, &session_id, Some("drop")).await;
        assert!(removed);

        let first_payload = tokio::time::timeout(Duration::from_secs(1), subscriber_rx.recv())
            .await
            .expect("queue update should be broadcast")
            .expect("queue update payload");
        assert_eq!(
            first_payload.get("type").and_then(Value::as_str),
            Some("queue_updated")
        );
        assert_eq!(
            first_payload
                .get("items")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );

        let cleared = cancel_queued_inputs(&state, &session_id, None).await;
        assert!(cleared);

        let second_payload = tokio::time::timeout(Duration::from_secs(1), subscriber_rx.recv())
            .await
            .expect("clear queue update should be broadcast")
            .expect("clear queue payload");
        assert_eq!(
            second_payload.get("type").and_then(Value::as_str),
            Some("queue_updated")
        );
        assert_eq!(
            second_payload
                .get("items")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );

        {
            let runs = state.runs.lock().await;
            assert!(runs.queues.get(&session_id).is_none());
        }
    }

    #[tokio::test]
    async fn queued_message_auto_dispatches_after_done() {
        let workspace = temp_workspace("queue-done");
        let state = test_state(&workspace, 8);
        let session_id = current_session_id(&state).await;
        let first_prompt = "slow-ok:first".to_string();
        let second_prompt = "second-after-done".to_string();

        submit_or_queue_message(&state, &session_id, first_prompt.clone())
            .await
            .expect("start first run");
        submit_or_queue_message(&state, &session_id, second_prompt.clone())
            .await
            .expect("queue second run");

        {
            let runs = state.runs.lock().await;
            assert_eq!(
                runs.queues
                    .get(&session_id)
                    .map(|queue| queue.len())
                    .unwrap_or(0),
                1
            );
        }

        let mut drained = false;
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let (has_active, queue_len) = {
                let runs = state.runs.lock().await;
                (
                    runs.active.contains_key(&session_id),
                    runs.queues
                        .get(&session_id)
                        .map(|queue| queue.len())
                        .unwrap_or(0),
                )
            };

            if !has_active && queue_len == 0 {
                drained = true;
                break;
            }
        }

        assert!(drained, "queued run should be drained after done");

        let history = load_session_history(&state, &session_id).await;
        let user_messages: Vec<&str> = history
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(
            user_messages,
            vec![first_prompt.as_str(), second_prompt.as_str()]
        );
        assert!(history.iter().any(|message| message.role == "assistant"
            && message.content == "mock-backend: slow-ok:first"));
        assert!(history.iter().any(|message| message.role == "assistant"
            && message.content == "mock-backend: second-after-done"));
    }

    #[tokio::test]
    async fn queued_message_is_not_auto_dispatched_after_error() {
        let workspace = temp_workspace("queue-error");
        let state = test_state(&workspace, 8);
        let session_id = current_session_id(&state).await;
        let first_prompt = "slow-fail:first".to_string();
        let second_prompt = "second-after-error".to_string();

        submit_or_queue_message(&state, &session_id, first_prompt.clone())
            .await
            .expect("start first run");
        submit_or_queue_message(&state, &session_id, second_prompt.clone())
            .await
            .expect("queue second run");

        let mut settled = false;
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let (has_active, queue_len) = {
                let runs = state.runs.lock().await;
                (
                    runs.active.contains_key(&session_id),
                    runs.queues
                        .get(&session_id)
                        .map(|queue| queue.len())
                        .unwrap_or(0),
                )
            };

            if !has_active && queue_len == 1 {
                settled = true;
                break;
            }
        }

        assert!(
            settled,
            "queue should remain queued when the active run ends with error"
        );

        let history = load_session_history(&state, &session_id).await;
        let user_messages: Vec<&str> = history
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(user_messages, vec![first_prompt.as_str()]);
        assert!(history.iter().any(|message| {
            message.role == "assistant"
                && message
                    .content
                    .contains("Error: mock failure for slow-fail:first")
        }));
    }

    #[tokio::test]
    async fn stop_active_run_clears_queue_and_persists_stop_note() {
        let workspace = temp_workspace("stop-clears-queue");
        let state = test_state(&workspace, 8);
        let session_id = current_session_id(&state).await;
        let first_prompt = "slow-ok:stop-target".to_string();

        submit_or_queue_message(&state, &session_id, first_prompt.clone())
            .await
            .expect("start first run");
        submit_or_queue_message(&state, &session_id, "queued-after-stop".to_string())
            .await
            .expect("queue second run");

        let stopped = stop_active_run(&state, "user_cancelled", Some(&session_id)).await;
        assert!(stopped);

        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let has_active = {
                let runs = state.runs.lock().await;
                runs.active.contains_key(&session_id)
            };
            if !has_active {
                break;
            }
        }

        {
            let runs = state.runs.lock().await;
            assert!(!runs.active.contains_key(&session_id));
            assert!(runs.queues.get(&session_id).is_none());
        }

        let history = load_session_history(&state, &session_id).await;
        assert!(history
            .iter()
            .any(|message| message.role == "assistant" && message.content == "Stopped by user."));
    }

    #[tokio::test]
    async fn reserve_or_queue_submission_queues_when_global_capacity_is_full() {
        let workspace = temp_workspace("capacity-queue");
        let state = test_state(&workspace, 1);
        let session_id = current_session_id(&state).await;
        let (subscriber_tx, mut subscriber_rx) = mpsc::unbounded_channel::<Value>();

        {
            let mut runs = state.runs.lock().await;
            runs.starting_sessions.insert("other-session".to_string());
            runs.subscribers.push(subscriber_tx);
        }

        let outcome =
            reserve_or_queue_submission(&state, &session_id, "queued-on-capacity".to_string())
                .await
                .expect("queue submission");
        assert!(matches!(outcome, SubmitOutcome::Queued));

        let payload = tokio::time::timeout(Duration::from_secs(1), subscriber_rx.recv())
            .await
            .expect("queue update should be broadcast")
            .expect("queue update payload");
        assert_eq!(
            payload.get("type").and_then(Value::as_str),
            Some("queue_updated")
        );
        assert_eq!(
            payload
                .get("session_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            Some(session_id.clone())
        );
    }

    #[tokio::test]
    async fn different_sessions_can_run_concurrently_when_capacity_allows() {
        let workspace = temp_workspace("parallel-sessions");
        let state = test_state(&workspace, 2);
        let session_a = current_session_id(&state).await;
        let session_b = create_session(&state, "parallel-b").await;
        let prompt_a = "slow-ok:parallel-a".to_string();
        let prompt_b = "slow-ok:parallel-b".to_string();

        submit_or_queue_message(&state, &session_a, prompt_a.clone())
            .await
            .expect("start first session run");
        submit_or_queue_message(&state, &session_b, prompt_b.clone())
            .await
            .expect("start second session run");

        let mut saw_parallel_active = false;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let active_len = {
                let runs = state.runs.lock().await;
                runs.active.len()
            };
            if active_len >= 2 {
                saw_parallel_active = true;
                break;
            }
        }
        assert!(
            saw_parallel_active,
            "both sessions should become active concurrently when cap allows"
        );

        let mut drained = false;
        for _ in 0..120 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let active_len = {
                let runs = state.runs.lock().await;
                runs.active.len()
            };
            if active_len == 0 {
                drained = true;
                break;
            }
        }
        assert!(drained, "parallel runs should eventually complete");

        let history_a = load_session_history(&state, &session_a).await;
        let history_b = load_session_history(&state, &session_b).await;
        assert!(history_a.iter().any(|message| {
            message.role == "assistant" && message.content == "mock-backend: slow-ok:parallel-a"
        }));
        assert!(history_b.iter().any(|message| {
            message.role == "assistant" && message.content == "mock-backend: slow-ok:parallel-b"
        }));
    }

    #[tokio::test]
    async fn queued_session_waiting_on_global_capacity_starts_after_done() {
        let workspace = temp_workspace("capacity-autostart");
        let state = test_state(&workspace, 1);
        let session_a = current_session_id(&state).await;
        let session_b = create_session(&state, "capacity-b").await;
        let prompt_a = "slow-ok:cap-a".to_string();
        let prompt_b = "cap-b-after-done".to_string();

        submit_or_queue_message(&state, &session_a, prompt_a.clone())
            .await
            .expect("start capacity owner run");
        submit_or_queue_message(&state, &session_b, prompt_b.clone())
            .await
            .expect("queue second session prompt");

        {
            let runs = state.runs.lock().await;
            assert_eq!(
                runs.queues
                    .get(&session_b)
                    .map(|queue| queue.len())
                    .unwrap_or(0),
                1
            );
        }

        let mut drained = false;
        for _ in 0..160 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let (has_active, queued_len) = {
                let runs = state.runs.lock().await;
                (
                    !runs.active.is_empty(),
                    runs.queues
                        .get(&session_b)
                        .map(|queue| queue.len())
                        .unwrap_or(0),
                )
            };
            if !has_active && queued_len == 0 {
                drained = true;
                break;
            }
        }
        assert!(
            drained,
            "queued session should auto-start after an active run finishes with done"
        );

        let history_b = load_session_history(&state, &session_b).await;
        let user_messages: Vec<&str> = history_b
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(user_messages, vec![prompt_b.as_str()]);
        assert!(history_b.iter().any(|message| {
            message.role == "assistant" && message.content == "mock-backend: cap-b-after-done"
        }));
    }

    #[tokio::test]
    async fn persist_agent_event_appends_tool_call_and_tool_result_messages() {
        let workspace = temp_workspace("persist-tool-events");
        let state = test_state(&workspace, 8);
        let session_id = current_session_id(&state).await;

        let tool_start = AgentEvent::ToolCallStart {
            call_id: "call-1".to_string(),
            name: "shell".to_string(),
            args: serde_json::json!({ "command": "echo hello" }),
        };
        persist_agent_event(&state, &session_id, &tool_start)
            .await
            .expect("persist tool call start");

        let tool_waiting = AgentEvent::ToolCallResult {
            call_id: "call-1".to_string(),
            name: "shell".to_string(),
            output: "blocked".to_string(),
            success: false,
            status: ToolCallStatus::Denied,
            approval_request_id: Some("req-1".to_string()),
            awaiting_approval: true,
        };
        persist_agent_event(&state, &session_id, &tool_waiting)
            .await
            .expect("skip awaiting approval result");

        let tool_result = AgentEvent::ToolCallResult {
            call_id: "call-1".to_string(),
            name: "shell".to_string(),
            output: "hello".to_string(),
            success: true,
            status: ToolCallStatus::Success,
            approval_request_id: None,
            awaiting_approval: false,
        };
        persist_agent_event(&state, &session_id, &tool_result)
            .await
            .expect("persist tool call result");

        let history = load_session_history(&state, &session_id).await;
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "assistant");
        assert_eq!(history[1].role, "tool");

        let assistant_payload: Value =
            serde_json::from_str(&history[0].content).expect("assistant payload as json");
        assert_eq!(
            assistant_payload
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            assistant_payload
                .pointer("/tool_calls/0/id")
                .and_then(Value::as_str),
            Some("call-1")
        );

        let tool_payload: Value =
            serde_json::from_str(&history[1].content).expect("tool payload as json");
        assert_eq!(
            tool_payload.get("tool_call_id").and_then(Value::as_str),
            Some("call-1")
        );
        assert_eq!(
            tool_payload.get("content").and_then(Value::as_str),
            Some("hello")
        );
        assert_eq!(
            tool_payload.get("status").and_then(Value::as_str),
            Some("success")
        );
    }
}
