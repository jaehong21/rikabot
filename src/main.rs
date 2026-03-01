mod agent;
mod config;
mod config_store;
mod gateway;
mod mcp_runtime;
mod permissions;
mod prompt;
mod providers;
mod session;
mod skills;
mod tools;

use anyhow::Result;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("rikabot=info".parse()?))
        .init();

    let config = config::AppConfig::load(None)?;
    let config_path = config::AppConfig::resolve_path(None)?;
    tracing::info!(
        "Loaded config: provider={}, model={}",
        config.provider,
        config.model
    );

    // Resolve workspace dir
    let workspace_dir = config.resolve_workspace_dir()?;

    // Create provider
    let provider: Box<dyn providers::Provider> = providers::create_provider(&config)?;

    let permissions_config = Arc::new(tokio::sync::RwLock::new(config.permissions.clone()));
    let permissions_engine = Arc::new(tokio::sync::RwLock::new(
        permissions::PermissionEngine::from_config(&config.permissions)?,
    ));

    // Create tool registry with default tools anchored to workspace.
    let tool_registry = tools::default_registry(&workspace_dir, permissions_engine.clone());
    let mcp_runtime = Arc::new(mcp_runtime::McpRuntime::new(
        config.mcp.enabled,
        &config.mcp.servers,
    ));
    if config.mcp.enabled && !config.mcp.servers.is_empty() {
        mcp_runtime.spawn_background(
            config.mcp.servers.clone(),
            workspace_dir.clone(),
            tool_registry.clone(),
        );
    }

    let prompt_manager = Arc::new(prompt::PromptManager::new(
        &workspace_dir,
        config.skills.enabled,
        prompt::PromptLimits {
            bootstrap_max_chars: config.prompt.bootstrap_max_chars,
            bootstrap_total_max_chars: config.prompt.bootstrap_total_max_chars,
        },
    )?);

    // Create agent
    let agent = Arc::new(agent::Agent::new(
        provider,
        tool_registry,
        config.model.clone(),
        config.temperature,
    ));

    // Create session manager
    let sessions = Arc::new(tokio::sync::Mutex::new(session::SessionManager::new(
        &workspace_dir,
    )?));

    let config_store = Arc::new(config_store::ConfigStore::new(config_path));

    // Start gateway
    tracing::info!("Starting server on http://{}:{}", config.host, config.port);
    gateway::serve(
        &config.host,
        config.port,
        agent,
        sessions,
        prompt_manager,
        permissions_config,
        permissions_engine,
        config_store,
        mcp_runtime,
    )
    .await?;

    Ok(())
}
