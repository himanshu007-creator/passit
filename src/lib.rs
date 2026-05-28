pub mod acp;
pub mod config;
pub mod db;
pub mod history;
pub mod server;
pub mod tools;

use config::Config;

/// Run the MCP server from a library context.
pub async fn run(config: Option<Config>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cfg = config.unwrap_or_else(Config::from_env);
    server::run_mcp_server(cfg).await
}
