mod agent;
mod cli;
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

#[tokio::main]
async fn main() -> Result<()> {
    cli::run().await
}
