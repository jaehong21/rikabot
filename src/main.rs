mod agent;
mod config;
mod gateway;
mod providers;
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

    // Build system prompt with skills
    let system_prompt = if config.skills.enabled {
        let workspace_dir = config
            .workspace_dir
            .as_deref()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".rika").join("workspace"))
            });
        let skills_dir = workspace_dir.map(|w| w.join("skills"));
        let loader = skills::SkillsLoader::new(skills_dir);
        let skills_section = loader.build_prompt_section();
        if skills_section.is_empty() {
            config.system_prompt.clone()
        } else {
            format!("{}\n\n---\n\n{}", config.system_prompt, skills_section)
        }
    } else {
        config.system_prompt.clone()
    };

    // Create agent
    let agent = Arc::new(agent::Agent::new(
        provider,
        tool_registry,
        system_prompt,
        config.model.clone(),
        config.temperature,
    ));

    // Start gateway
    tracing::info!("Starting server on {}:{}", config.host, config.port);
    gateway::serve(&config.host, config.port, agent).await?;

    Ok(())
}
