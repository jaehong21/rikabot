use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

use crate::agent;
use crate::config;
use crate::config_store;
use crate::gateway;
use crate::mcp_runtime;
use crate::permissions;
use crate::prompt;
use crate::providers;
use crate::session;
use crate::tools;

#[derive(Debug, Parser)]
#[command(
    name = "rika",
    version,
    about = "Rika server CLI",
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    /// Override config path (default: ~/.rika/config.toml or RIKA_CONFIG env)
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start server (default: background daemon mode)
    Start {
        /// Run in foreground instead of daemon mode
        #[arg(long, default_value_t = false)]
        foreground: bool,
    },
    /// Restart running daemon
    Restart,
    /// Stop running daemon
    Stop,
    /// Show daemon status
    Status,
    /// Internal foreground server command used by lifecycle commands
    #[command(hide = true)]
    Serve,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeState {
    pid: u32,
    host: String,
    port: u16,
    config_path: String,
}

struct RuntimeContext {
    config: config::AppConfig,
    config_path: PathBuf,
    runtime_state_path: PathBuf,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve => run_server(cli.config.as_deref()).await,
        Commands::Start { foreground } => start_server(cli.config.as_deref(), foreground).await,
        Commands::Restart => restart_server(cli.config.as_deref()).await,
        Commands::Stop => {
            let _ = stop_server(cli.config.as_deref(), false).await?;
            Ok(())
        }
        Commands::Status => status_server(cli.config.as_deref()).await,
    }
}

async fn start_server(config_arg: Option<&str>, foreground: bool) -> Result<()> {
    if foreground {
        return run_server(config_arg).await;
    }

    let ctx = load_runtime_context(config_arg)?;

    if let Some(existing) = read_runtime_state(&ctx.runtime_state_path)? {
        if process_is_running(existing.pid) {
            let healthy = is_server_healthy(&ctx.config.host, ctx.config.port).await;
            let url = format!("http://{}:{}", ctx.config.host, ctx.config.port);
            if healthy {
                println!(
                    "Server is already running (pid: {}, url: {}).",
                    existing.pid, url
                );
            } else {
                println!(
                    "Server process is running (pid: {}) but health check failed (url: {}).",
                    existing.pid, url
                );
            }
            return Ok(());
        }

        remove_runtime_state(&ctx.runtime_state_path)?;
    }

    let current_exe = std::env::current_exe().context("failed to resolve current executable")?;
    let child = Command::new(current_exe)
        .arg("--config")
        .arg(&ctx.config_path)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn daemon process")?;

    let state = RuntimeState {
        pid: child.id(),
        host: ctx.config.host.clone(),
        port: ctx.config.port,
        config_path: ctx.config_path.to_string_lossy().to_string(),
    };
    write_runtime_state(&ctx.runtime_state_path, &state)?;

    let url = format!("http://{}:{}", ctx.config.host, ctx.config.port);
    for _ in 0..20 {
        if !process_is_running(state.pid) {
            remove_runtime_state(&ctx.runtime_state_path)?;
            anyhow::bail!("daemon process exited before becoming healthy");
        }
        if is_server_healthy(&ctx.config.host, ctx.config.port).await {
            println!("Server started (pid: {}, url: {}).", state.pid, url);
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    println!(
        "Server started (pid: {}, url: {}), but health check timed out.",
        state.pid, url
    );

    Ok(())
}

async fn restart_server(config_arg: Option<&str>) -> Result<()> {
    let _ = stop_server(config_arg, true).await?;
    start_server(config_arg, false).await
}

async fn stop_server(config_arg: Option<&str>, quiet_when_not_running: bool) -> Result<bool> {
    let ctx = load_runtime_context(config_arg)?;
    let Some(state) = read_runtime_state(&ctx.runtime_state_path)? else {
        if !quiet_when_not_running {
            println!("Server is not running.");
        }
        return Ok(false);
    };

    if !process_is_running(state.pid) {
        remove_runtime_state(&ctx.runtime_state_path)?;
        if !quiet_when_not_running {
            println!("Server is not running (removed stale runtime state).");
        }
        return Ok(false);
    }

    terminate_process(state.pid)?;

    for _ in 0..40 {
        if !process_is_running(state.pid) {
            remove_runtime_state(&ctx.runtime_state_path)?;
            println!("Server stopped (pid: {}).", state.pid);
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    anyhow::bail!(
        "timed out while stopping pid {}; process is still running",
        state.pid
    );
}

async fn status_server(config_arg: Option<&str>) -> Result<()> {
    let ctx = load_runtime_context(config_arg)?;
    let health = is_server_healthy(&ctx.config.host, ctx.config.port).await;
    let url = format!("http://{}:{}", ctx.config.host, ctx.config.port);

    match read_runtime_state(&ctx.runtime_state_path)? {
        Some(state) if process_is_running(state.pid) => {
            println!("status: running");
            println!("pid: {}", state.pid);
            println!("url: {}", url);
            println!("health: {}", if health { "ok" } else { "unhealthy" });
            println!("config: {}", state.config_path);
        }
        Some(_) => {
            remove_runtime_state(&ctx.runtime_state_path)?;
            println!("status: stopped");
            println!("url: {}", url);
            println!(
                "health: {}",
                if health {
                    "ok (unmanaged process)"
                } else {
                    "unreachable"
                }
            );
            println!("note: stale runtime state was removed");
        }
        None => {
            if health {
                println!("status: running (unmanaged)");
                println!("url: {}", url);
                println!("health: ok");
            } else {
                println!("status: stopped");
                println!("url: {}", url);
                println!("health: unreachable");
            }
        }
    }

    Ok(())
}

fn load_runtime_context(config_arg: Option<&str>) -> Result<RuntimeContext> {
    let config_path = config::AppConfig::resolve_path(config_arg)?;
    let config = config::AppConfig::load_from_path(&config_path)?;
    let runtime_dir = config.resolve_workspace_dir()?.join("runtime");
    fs::create_dir_all(&runtime_dir)
        .with_context(|| format!("failed to create runtime dir {}", runtime_dir.display()))?;

    Ok(RuntimeContext {
        config,
        config_path,
        runtime_state_path: runtime_dir.join("server.json"),
    })
}

fn read_runtime_state(path: &Path) -> Result<Option<RuntimeState>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read runtime state {}", path.display()))?;
    let state = serde_json::from_str::<RuntimeState>(&raw)
        .with_context(|| format!("failed to parse runtime state {}", path.display()))?;

    Ok(Some(state))
}

fn write_runtime_state(path: &Path, state: &RuntimeState) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create runtime dir {}", parent.display()))?;
    }

    let payload = serde_json::to_vec_pretty(state).context("failed to serialize runtime state")?;
    fs::write(path, payload)
        .with_context(|| format!("failed to write runtime state {}", path.display()))
}

fn remove_runtime_state(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove runtime state {}", path.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> Result<()> {
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .with_context(|| format!("failed to send SIGTERM to pid {}", pid))?;

    if !status.success() {
        anyhow::bail!("failed to stop pid {} via SIGTERM", pid);
    }

    Ok(())
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    let filter = format!("PID eq {}", pid);
    let output = match Command::new("tasklist")
        .args(["/FI", &filter])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(out) => out,
        Err(_) => return false,
    };

    if !output.status.success() {
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|line| line.contains(&pid.to_string()))
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<()> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .with_context(|| format!("failed to terminate pid {}", pid))?;

    if !status.success() {
        anyhow::bail!("failed to terminate pid {}", pid);
    }

    Ok(())
}

async fn is_server_healthy(host: &str, port: u16) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    let url = format!("http://{}:{}/health", host, port);
    match client.get(url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

async fn run_server(config_arg: Option<&str>) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = config::AppConfig::load(config_arg)?;
    let config_path = config::AppConfig::resolve_path(config_arg)?;
    tracing::info!(
        "Loaded config: provider={}, model={}",
        config.provider,
        config.model
    );

    let workspace_dir = config.resolve_workspace_dir()?;

    let provider: Box<dyn providers::Provider> = providers::create_provider(&config)?;

    let permissions_config = Arc::new(tokio::sync::RwLock::new(config.permissions.clone()));
    let permissions_engine = Arc::new(tokio::sync::RwLock::new(
        permissions::PermissionEngine::from_config(&config.permissions)?,
    ));

    let tool_registry = tools::default_registry(
        &workspace_dir,
        permissions_engine.clone(),
        &config.shell,
        &config.process,
        &config.web_fetch,
        &config.web_search,
    );
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

    let agent = Arc::new(agent::Agent::new(
        provider,
        tool_registry,
        config.model.clone(),
        config.temperature,
    ));

    let sessions = Arc::new(tokio::sync::Mutex::new(session::SessionManager::new(
        &workspace_dir,
    )?));

    let config_store = Arc::new(config_store::ConfigStore::new(config_path));

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

#[cfg(test)]
mod tests {
    use super::{Cli, Commands};
    use clap::Parser;

    #[test]
    fn cli_parses_start_foreground() {
        let cli = Cli::try_parse_from(["rika", "start", "--foreground"]).unwrap();
        match cli.command {
            Commands::Start { foreground } => assert!(foreground),
            other => panic!("expected start command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_restart() {
        let cli = Cli::try_parse_from(["rika", "restart"]).unwrap();
        match cli.command {
            Commands::Restart => {}
            other => panic!("expected restart command, got {other:?}"),
        }
    }

    #[test]
    fn cli_parses_global_config() {
        let cli = Cli::try_parse_from(["rika", "--config", "/tmp/rika.toml", "status"]).unwrap();
        assert_eq!(cli.config.as_deref(), Some("/tmp/rika.toml"));
    }

    #[test]
    fn cli_parses_no_subcommand() {
        let err = Cli::try_parse_from(["rika"]).unwrap_err();
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }
}
