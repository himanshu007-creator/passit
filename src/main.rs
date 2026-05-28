use std::sync::Arc;

use clap::{Parser, Subcommand};
use rmcp::model::RawContent;

mod acp;
mod config;
mod db;
mod history;
mod server;
mod tools;

#[derive(Parser)]
#[command(name = "passit")]
#[command(about = "Pass conversations between AI agents — grab, drop, and continue seamlessly")]
#[command(version, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the MCP server on stdio (default)
    Server {
        #[arg(long, env = "PASSIT_DB_PATH")]
        db_path: Option<String>,

        #[arg(long, env = "PASSIT_AGENT_ID")]
        agent_id: Option<String>,
    },
    /// Enable ACP REST server alongside MCP
    #[command(name = "acp")]
    Acp {
        #[arg(long, default_value = "7700")]
        port: u16,

        #[arg(long, env = "PASSIT_DB_PATH")]
        db_path: Option<String>,

        #[arg(long, env = "PASSIT_AGENT_ID")]
        agent_id: Option<String>,
    },
    /// Export a session to JSON
    Export {
        session_id: String,

        #[arg(long, default_value = "json")]
        format: String,

        #[arg(long, env = "PASSIT_DB_PATH")]
        db_path: Option<String>,
    },
    /// List recent sessions
    List {
        #[arg(long)]
        limit: Option<u32>,

        #[arg(long)]
        agent: Option<String>,

        #[arg(long, env = "PASSIT_DB_PATH")]
        db_path: Option<String>,
    },
    /// Scan agent history files and import past conversations
    Scan {
        #[arg(long, env = "PASSIT_DB_PATH")]
        db_path: Option<String>,

        #[arg(long, help = "Show detailed per-scanner breakdown")]
        verbose: bool,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        None | Some(Commands::Server { .. }) => {
            let cfg = config::Config::from_env();
            server::run_mcp_server(cfg).await?;
        }
        Some(Commands::Acp {
            port,
            db_path,
            agent_id,
        }) => {
            let mut cfg = config::Config::from_env();
            cfg.enable_acp = true;
            if let Some(p) = db_path {
                cfg.db_path = p.into();
            }
            if let Some(id) = agent_id {
                cfg.agent_id = id;
            }
            cfg.acp_port = port;

            let db = Arc::new(db::database::Database::open(&cfg.db_path)?);

            // Start ACP server in background
            let db_clone = db.clone();
            let acp_cfg = cfg.clone();
            let acp_handle =
                tokio::spawn(
                    async move { acp::server::start_acp_server(db_clone, &acp_cfg).await },
                );

            // Start MCP server on stdio
            server::run_mcp_server(cfg).await?;

            acp_handle
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)??;
        }
        Some(Commands::Export {
            session_id,
            format,
            db_path,
        }) => {
            let db_path = db_path.map(Into::into).unwrap_or_else(|| {
                let cfg = config::Config::from_env();
                cfg.db_path
            });
            let db = db::database::Database::open(&db_path)?;

            let result = tools::export_tool::export_session(
                &db,
                tools::export_tool::ExportSessionParams {
                    session_id,
                    format: Some(format),
                },
            )
            .await?;

            for content in result.content {
                if let RawContent::Text(text) = &content.raw {
                    println!("{}", text.text);
                }
            }
        }
        Some(Commands::List {
            limit,
            agent,
            db_path,
        }) => {
            let db_path = db_path.map(Into::into).unwrap_or_else(|| {
                let cfg = config::Config::from_env();
                cfg.db_path
            });
            let db = db::database::Database::open(&db_path)?;

            let result = tools::list::list_sessions_tool(
                &db,
                tools::list::ListSessionsParams {
                    limit,
                    offset: None,
                    project_path: None,
                    agent,
                    tag: None,
                    since: None,
                    source: None,
                },
            )
            .await?;

            for content in result.content {
                if let RawContent::Text(text) = &content.raw {
                    println!("{}", text.text);
                }
            }
        }
        Some(Commands::Scan { db_path, verbose }) => {
            let db_path = db_path.map(Into::into).unwrap_or_else(|| {
                let cfg = config::Config::from_env();
                cfg.db_path
            });
            let db = db::database::Database::open(&db_path)?;

            let summary = history::run_history_scanners(&db);
            println!("History scan complete:");
            println!(
                "  Total imported: {} sessions ({} messages)",
                summary.total_sessions, summary.total_messages
            );
            if verbose {
                for s in &summary.scanners {
                    println!(
                        "  {}: {} found, {} imported ({} messages)",
                        s.name, s.sessions_found, s.sessions_imported, s.messages_imported
                    );
                }
            } else {
                for s in &summary.scanners {
                    if s.sessions_imported > 0 {
                        println!("  {}: {} imported", s.name, s.sessions_imported);
                    }
                }
            }
        }
    }

    Ok(())
}
