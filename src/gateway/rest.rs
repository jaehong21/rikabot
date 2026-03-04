use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::config::{PermissionsConfig, ToolPermissionsConfig};
use crate::gateway::{apply_permissions_update, AppState};
use crate::providers::ChatMessage;
use crate::session::SessionRecord;
use crate::skills::{self, SkillsStatusSnapshot};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/threads", get(list_threads).post(create_thread))
        .route(
            "/threads/{session_id}",
            patch(rename_thread).delete(delete_thread),
        )
        .route(
            "/threads/{session_id}/messages",
            get(get_thread_messages).delete(clear_thread_messages),
        )
        .route(
            "/settings/permissions",
            get(get_permissions).put(update_permissions),
        )
        .route("/settings/skills", get(get_skills))
        .route(
            "/settings/skills/content",
            get(read_skill_content).put(update_skill_content),
        )
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct ApiErrorPayload {
    error: ApiErrorBody,
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    code: String,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    fn unprocessable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "validation_error",
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorPayload {
                error: ApiErrorBody {
                    code: self.code.to_string(),
                    message: self.message,
                },
            }),
        )
            .into_response()
    }
}

fn map_api_error(err: anyhow::Error) -> ApiError {
    let message = err.to_string();
    if message.contains("session not found") || message.contains("current session not found") {
        return ApiError::not_found(message);
    }
    if message.contains("invalid rule")
        || message.contains("allow_rule cannot be empty")
        || message.contains("frontmatter")
        || message.contains("only SKILL.md files can be edited")
        || message.contains("skill path must remain under workspace skills directory")
    {
        return ApiError::unprocessable(message);
    }
    if message.contains("missing required field")
        || message.contains("must not be empty")
        || message.contains("cannot be empty")
    {
        return ApiError::bad_request(message);
    }
    ApiError::internal(message)
}

#[derive(Debug, Serialize)]
struct ThreadsResponse {
    sessions: Vec<SessionRecord>,
}

#[derive(Debug, Deserialize)]
struct CreateThreadRequest {
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateThreadResponse {
    session: SessionRecord,
    sessions: Vec<SessionRecord>,
}

#[derive(Debug, Deserialize)]
struct RenameThreadRequest {
    display_name: String,
}

#[derive(Debug, Serialize)]
struct RenameThreadResponse {
    session: SessionRecord,
    sessions: Vec<SessionRecord>,
}

#[derive(Debug, Serialize)]
struct ThreadMessagesResponse {
    session_id: String,
    history: Vec<ChatMessage>,
}

#[derive(Debug, Serialize)]
struct DeleteThreadResponse {
    deleted_session_id: String,
    fallback_session_id: String,
    sessions: Vec<SessionRecord>,
}

#[derive(Debug, Serialize)]
struct PermissionsResponse {
    permissions: PermissionsConfig,
}

#[derive(Debug, Deserialize)]
struct UpdatePermissionsRequest {
    enabled: bool,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SkillsResponse {
    skills: SkillsStatusSnapshot,
}

#[derive(Debug, Deserialize)]
struct SkillContentQuery {
    path: String,
}

#[derive(Debug, Serialize)]
struct SkillContentResponse {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct UpdateSkillContentRequest {
    path: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct UpdateSkillContentResponse {
    skills: SkillsStatusSnapshot,
    path: String,
    content: String,
}

async fn list_threads(State(state): State<AppState>) -> ApiResult<Json<ThreadsResponse>> {
    let sessions = {
        let mut sessions = state.sessions.lock().await;
        sessions.reload_from_disk().map_err(map_api_error)?;
        sessions.list_sessions()
    };

    Ok(Json(ThreadsResponse { sessions }))
}

async fn create_thread(
    State(state): State<AppState>,
    Json(body): Json<CreateThreadRequest>,
) -> ApiResult<Json<CreateThreadResponse>> {
    let (session, sessions) = {
        let mut sessions = state.sessions.lock().await;
        sessions.reload_from_disk().map_err(map_api_error)?;
        let session = sessions
            .create_session(body.display_name.as_deref())
            .map_err(map_api_error)?;
        (session, sessions.list_sessions())
    };

    broadcast_thread_list_update(&state).await;
    Ok(Json(CreateThreadResponse { session, sessions }))
}

async fn rename_thread(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<RenameThreadRequest>,
) -> ApiResult<Json<RenameThreadResponse>> {
    let (session, sessions) = {
        let mut sessions = state.sessions.lock().await;
        sessions.reload_from_disk().map_err(map_api_error)?;
        let session = sessions
            .rename_session(&session_id, &body.display_name)
            .map_err(map_api_error)?;
        (session, sessions.list_sessions())
    };

    broadcast_thread_list_update(&state).await;
    Ok(Json(RenameThreadResponse { session, sessions }))
}

async fn get_thread_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<ThreadMessagesResponse>> {
    let history = {
        let mut sessions = state.sessions.lock().await;
        sessions.reload_from_disk().map_err(map_api_error)?;
        if sessions.get_session(&session_id).is_none() {
            return Err(ApiError::not_found(format!(
                "session not found: {}",
                session_id
            )));
        }
        sessions.load_history(&session_id).map_err(map_api_error)?
    };

    Ok(Json(ThreadMessagesResponse {
        session_id,
        history,
    }))
}

async fn clear_thread_messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<ThreadMessagesResponse>> {
    {
        let mut sessions = state.sessions.lock().await;
        sessions.reload_from_disk().map_err(map_api_error)?;
        sessions.clear_session(&session_id).map_err(map_api_error)?;
    }

    broadcast_thread_list_update(&state).await;
    Ok(Json(ThreadMessagesResponse {
        session_id,
        history: Vec::new(),
    }))
}

async fn delete_thread(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<DeleteThreadResponse>> {
    let (deleted_session_id, fallback_session_id, sessions) = {
        let mut sessions = state.sessions.lock().await;
        sessions.reload_from_disk().map_err(map_api_error)?;
        let deleted = sessions
            .delete_session(&session_id)
            .map_err(map_api_error)?;
        (
            deleted.deleted_session_id,
            deleted.current_session_id,
            sessions.list_sessions(),
        )
    };

    broadcast_thread_list_update(&state).await;
    Ok(Json(DeleteThreadResponse {
        deleted_session_id,
        fallback_session_id,
        sessions,
    }))
}

async fn get_permissions(State(state): State<AppState>) -> ApiResult<Json<PermissionsResponse>> {
    let permissions = {
        let current = state.permissions_config.read().await;
        current.clone()
    };

    Ok(Json(PermissionsResponse { permissions }))
}

async fn update_permissions(
    State(state): State<AppState>,
    Json(body): Json<UpdatePermissionsRequest>,
) -> ApiResult<Json<PermissionsResponse>> {
    let next = PermissionsConfig {
        enabled: body.enabled,
        tools: ToolPermissionsConfig {
            allow: sanitize_rules(&body.allow),
            deny: sanitize_rules(&body.deny),
        },
    };

    apply_permissions_update(&state, next.clone())
        .await
        .map_err(map_api_error)?;

    Ok(Json(PermissionsResponse { permissions: next }))
}

async fn get_skills(State(state): State<AppState>) -> ApiResult<Json<SkillsResponse>> {
    let skills_dir = state.prompt_manager.workspace_dir().join("skills");
    let skills =
        skills::build_skills_status_snapshot(&skills_dir, state.prompt_manager.skills_enabled());

    Ok(Json(SkillsResponse { skills }))
}

async fn read_skill_content(
    State(state): State<AppState>,
    Query(query): Query<SkillContentQuery>,
) -> ApiResult<Json<SkillContentResponse>> {
    if query.path.trim().is_empty() {
        return Err(ApiError::bad_request("missing required field 'path'"));
    }

    let skills_dir = state.prompt_manager.workspace_dir().join("skills");
    let (path, content) =
        skills::read_skill_file(&skills_dir, &query.path).map_err(map_api_error)?;

    Ok(Json(SkillContentResponse {
        path: path.display().to_string(),
        content,
    }))
}

async fn update_skill_content(
    State(state): State<AppState>,
    Json(body): Json<UpdateSkillContentRequest>,
) -> ApiResult<Json<UpdateSkillContentResponse>> {
    let skills_dir = state.prompt_manager.workspace_dir().join("skills");
    skills::write_skill_file(&skills_dir, &body.path, &body.content).map_err(map_api_error)?;
    let (path, content) =
        skills::read_skill_file(&skills_dir, &body.path).map_err(map_api_error)?;
    let skills =
        skills::build_skills_status_snapshot(&skills_dir, state.prompt_manager.skills_enabled());

    Ok(Json(UpdateSkillContentResponse {
        skills,
        path: path.display().to_string(),
        content,
    }))
}

fn sanitize_rules(raw: &[String]) -> Vec<String> {
    raw.iter()
        .map(|rule| rule.trim())
        .filter(|rule| !rule.is_empty())
        .map(ToString::to_string)
        .collect()
}

async fn broadcast_thread_list_update(state: &AppState) {
    let payload = {
        let mut sessions = state.sessions.lock().await;
        if let Err(err) = sessions.reload_from_disk() {
            tracing::warn!("failed to reload sessions for thread broadcast: {}", err);
            return;
        }
        serde_json::json!({
            "type": "thread_list",
            "sessions": sessions.list_sessions(),
        })
    };
    let _ = state.thread_events.send(payload);
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
        let dir =
            std::env::temp_dir().join(format!("rikabot-rest-test-{}-{}", name, Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    fn test_state(workspace: &PathBuf, skills_enabled: bool) -> AppState {
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
                skills_enabled,
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

    #[tokio::test]
    async fn threads_endpoints_support_crud_flow() {
        let workspace = temp_workspace("threads");
        let state = test_state(&workspace, false);

        let listed = list_threads(State(state.clone())).await.expect("list").0;
        assert!(!listed.sessions.is_empty());

        let created = create_thread(
            State(state.clone()),
            Json(CreateThreadRequest {
                display_name: Some("alpha".to_string()),
            }),
        )
        .await
        .expect("create")
        .0;
        let sid = created.session.id.clone();

        let renamed = rename_thread(
            State(state.clone()),
            Path(sid.clone()),
            Json(RenameThreadRequest {
                display_name: "renamed".to_string(),
            }),
        )
        .await
        .expect("rename")
        .0;
        assert_eq!(renamed.session.display_name, "renamed");

        {
            let mut sessions = state.sessions.lock().await;
            sessions
                .append_messages(&sid, &[ChatMessage::user("hello")])
                .expect("append");
        }

        let history = get_thread_messages(State(state.clone()), Path(sid.clone()))
            .await
            .expect("history")
            .0;
        assert_eq!(history.history.len(), 1);

        let not_found = get_thread_messages(
            State(state.clone()),
            Path("00000000-0000-0000-0000-000000000000".to_string()),
        )
        .await
        .expect_err("unknown session should fail")
        .into_response();
        assert_eq!(not_found.status(), StatusCode::NOT_FOUND);

        let cleared = clear_thread_messages(State(state.clone()), Path(sid.clone()))
            .await
            .expect("clear")
            .0;
        assert!(cleared.history.is_empty());

        let deleted = delete_thread(State(state.clone()), Path(sid.clone()))
            .await
            .expect("delete")
            .0;
        assert_eq!(deleted.deleted_session_id, sid);
        assert!(!deleted.sessions.is_empty());
    }

    #[tokio::test]
    async fn permissions_endpoints_support_get_and_put() {
        let workspace = temp_workspace("permissions");
        let state = test_state(&workspace, false);

        let current = get_permissions(State(state.clone())).await.expect("get").0;
        assert!(!current.permissions.enabled);

        let updated = update_permissions(
            State(state.clone()),
            Json(UpdatePermissionsRequest {
                enabled: true,
                allow: vec!["shell(command:echo ok *)".to_string()],
                deny: vec![],
            }),
        )
        .await
        .expect("put")
        .0;
        assert!(updated.permissions.enabled);

        let err = update_permissions(
            State(state.clone()),
            Json(UpdatePermissionsRequest {
                enabled: true,
                allow: vec!["not-a-valid-rule".to_string()],
                deny: vec![],
            }),
        )
        .await
        .expect_err("invalid rule should fail")
        .into_response();
        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn skills_endpoints_support_read_and_write() {
        let workspace = temp_workspace("skills");
        let state = test_state(&workspace, true);

        let skill_dir = workspace.join("skills").join("demo-skill");
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        let skill_path = skill_dir.join("SKILL.md");
        fs::write(
            &skill_path,
            "---\nname: demo-skill\ndescription: before\n---\n\n# Demo",
        )
        .expect("seed skill");

        let status = get_skills(State(state.clone())).await.expect("skills").0;
        assert!(status.skills.skills.iter().any(|s| s.name == "demo-skill"));

        let read = read_skill_content(
            State(state.clone()),
            Query(SkillContentQuery {
                path: "demo-skill/SKILL.md".to_string(),
            }),
        )
        .await
        .expect("read")
        .0;
        assert!(read.content.contains("description: before"));

        let updated = update_skill_content(
            State(state.clone()),
            Json(UpdateSkillContentRequest {
                path: "demo-skill/SKILL.md".to_string(),
                content: "---\nname: demo-skill\ndescription: after\n---\n\n# Demo".to_string(),
            }),
        )
        .await
        .expect("update")
        .0;
        assert!(updated.content.contains("description: after"));

        let err = update_skill_content(
            State(state.clone()),
            Json(UpdateSkillContentRequest {
                path: "demo-skill/SKILL.md".to_string(),
                content: "---\nname: demo-skill\n---\n\n# Demo".to_string(),
            }),
        )
        .await
        .expect_err("invalid frontmatter should fail")
        .into_response();
        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
