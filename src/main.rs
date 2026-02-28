mod agent;
mod config;
mod gateway;
mod prompt;
mod providers;
mod session;
mod skills;
mod tools;

use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("rikabot=info".parse()?))
        .init();

    let config = config::AppConfig::load(None)?;
    tracing::info!(
        "Loaded config: provider={}, model={}",
        config.provider,
        config.model
    );

    // Create provider
    let provider: Box<dyn providers::Provider> = providers::create_provider(&config)?;

    // Create tool registry with default tools
    let tool_registry = tools::default_registry();

    // Resolve workspace dir
    let workspace_dir = config
        .workspace_dir
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".rika").join("workspace"))
        })
        .ok_or_else(|| {
            anyhow::anyhow!("workspace_dir could not be resolved from config or HOME")
        })?;

    let prompt_manager = Arc::new(prompt::PromptManager::new(
        &workspace_dir,
        config.system_prompt.clone(),
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

    // Start gateway
    tracing::info!("Starting server on http://{}:{}", config.host, config.port);
    gateway::serve(&config.host, config.port, agent, sessions, prompt_manager).await?;

    Ok(())
}
