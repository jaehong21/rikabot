mod config;
mod providers;
mod tools;
mod agent;
mod gateway;

use anyhow::Result;
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
        config.provider.kind,
        config.provider.model
    );

    // Create provider
    let provider: Box<dyn providers::Provider> = providers::create_provider(&config.provider)?;

    // Create tool registry with default tools
    let tool_registry = tools::default_registry();

    // Create agent
    let agent = Arc::new(agent::Agent::new(
        provider,
        tool_registry,
        config.system_prompt.clone(),
        config.provider.model.clone(),
        config.provider.temperature,
    ));

    // Start gateway
    tracing::info!("Starting server on {}:{}", config.host, config.port);
    gateway::serve(&config.host, config.port, agent).await?;

    Ok(())
}
