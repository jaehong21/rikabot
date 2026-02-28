use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::agent::AgentEvent;
use crate::gateway::AppState;
use crate::providers::ChatMessage;

/// Inbound message from the client.
#[derive(Debug, Clone, Deserialize)]
struct ClientMessage {
    #[serde(rename = "type")]
    msg_type: String,
    content: Option<String>,
    session_id: Option<String>,
    display_name: Option<String>,
}

struct RunOutcome {
    result: anyhow::Result<()>,
    updated_history: Vec<ChatMessage>,
    previous_len: usize,
}

enum RunSignal {
    Event { run_id: u64, event: AgentEvent },
    Completed { run_id: u64, outcome: RunOutcome },
}

struct ActiveRun {
    run_id: u64,
    session_id: String,
    agent_task: JoinHandle<()>,
    event_task: JoinHandle<()>,
}

/// Axum handler that upgrades an HTTP request to a WebSocket connection.
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle a single WebSocket connection.
async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut ws_sink, mut ws_stream) = socket.split();
    let (run_signal_tx, mut run_signal_rx) = mpsc::unbounded_channel::<RunSignal>();

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

    let mut active_run: Option<ActiveRun> = None;
    let mut next_run_id: u64 = 1;

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
                        if active_run.is_some() {
                            let _ = send_error(&mut ws_sink, "A run is already active. Stop it before sending another message.").await;
                            continue;
                        }

                        let content = match client_msg.content {
                            Some(c) if !c.trim().is_empty() => c,
                            _ => continue,
                        };

                        match spawn_active_run(
                            &state,
                            run_signal_tx.clone(),
                            next_run_id,
                            &current_session_id,
                            &history,
                            content,
                        ) {
                            Ok(run) => {
                                active_run = Some(run);
                                next_run_id = next_run_id.saturating_add(1);
                            }
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
                        if let Some(run) = active_run.take() {
                            let stopped_session = run.session_id.clone();
                            abort_active_run(run).await;
                            let _ = send_stopped(&mut ws_sink, "user_cancelled", Some(&stopped_session)).await;
                        } else {
                            let _ = send_stopped(&mut ws_sink, "no_active_run", Some(&current_session_id)).await;
                        }
                    }
                    _ => {
                        if active_run.is_some() && is_thread_mutating_command(&client_msg.msg_type) {
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
                                if ws_sink
                                    .send(Message::text(event.to_string()))
                                    .await
                                    .is_err()
                                {
                                    break;
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
                let Some(signal) = signal else {
                    break;
                };

                let should_continue = handle_run_signal(
                    signal,
                    &mut active_run,
                    &mut ws_sink,
                    &state,
                    &current_session_id,
                    &mut history,
                )
                .await;

                if !should_continue {
                    break;
                }
            }
        }
    }

    if let Some(run) = active_run.take() {
        abort_active_run(run).await;
    }

    tracing::debug!("WebSocket connection closed");
}

fn spawn_active_run(
    state: &AppState,
    run_signal_tx: mpsc::UnboundedSender<RunSignal>,
    run_id: u64,
    session_id: &str,
    history: &[ChatMessage],
    content: String,
) -> anyhow::Result<ActiveRun> {
    let system_prompt = state.prompt_manager.build_prompt()?;
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (outcome_tx, outcome_rx) = oneshot::channel::<RunOutcome>();
    let agent = state.agent.clone();
    let mut history_clone = history.to_vec();
    let previous_len = history_clone.len();

    let agent_task = tokio::spawn(async move {
        let result = agent
            .run(system_prompt, &mut history_clone, content, event_tx)
            .await;
        let _ = outcome_tx.send(RunOutcome {
            result,
            updated_history: history_clone,
            previous_len,
        });
    });

    let event_task_tx = run_signal_tx;
    let event_task = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if event_task_tx
                .send(RunSignal::Event { run_id, event })
                .is_err()
            {
                break;
            }
        }

        if let Ok(outcome) = outcome_rx.await {
            let _ = event_task_tx.send(RunSignal::Completed { run_id, outcome });
        }
    });

    Ok(ActiveRun {
        run_id,
        session_id: session_id.to_string(),
        agent_task,
        event_task,
    })
}

async fn abort_active_run(run: ActiveRun) {
    run.agent_task.abort();
    run.event_task.abort();
    let _ = run.agent_task.await;
    let _ = run.event_task.await;
}

async fn handle_run_signal(
    signal: RunSignal,
    active_run: &mut Option<ActiveRun>,
    ws_sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
    current_session_id: &str,
    history: &mut Vec<ChatMessage>,
) -> bool {
    match signal {
        RunSignal::Event { run_id, event } => {
            if active_run.as_ref().map_or(true, |run| run.run_id != run_id) {
                return true;
            }

            let json = match serde_json::to_string(&event) {
                Ok(j) => j,
                Err(_) => return true,
            };
            if ws_sink.send(Message::text(json)).await.is_err() {
                return false;
            }
        }
        RunSignal::Completed { run_id, outcome } => {
            if active_run.as_ref().map_or(true, |run| run.run_id != run_id) {
                return true;
            }

            let finished_run = active_run.take().expect("active run exists");
            let run_session_id = finished_run.session_id.clone();

            let _ = finished_run.agent_task.await;
            let _ = finished_run.event_task.await;

            match outcome.result {
                Ok(()) => {
                    if outcome.previous_len <= outcome.updated_history.len() {
                        let appended = &outcome.updated_history[outcome.previous_len..];
                        let append_result = {
                            let mut sessions = state.sessions.lock().await;
                            sessions.append_messages(&run_session_id, appended)
                        };
                        if let Err(err) = append_result {
                            let _ = send_error(
                                ws_sink,
                                &format!("Failed to persist session messages: {}", err),
                            )
                            .await;
                        }
                    }

                    if run_session_id == current_session_id {
                        *history = outcome.updated_history;
                    }
                    let _ = send_thread_list(ws_sink, state).await;
                }
                Err(e) => {
                    let _ = send_error(ws_sink, &format!("Agent error: {}", e)).await;
                }
            }
        }
    }

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
