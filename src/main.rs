mod config;
mod client;
mod crypto;
mod fileops;
mod frontend;
mod protocol;
mod server;

use clap::{Parser, Subcommand};
use config::AppConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "drift", about = "Encrypted file transfer over WebSocket")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    // Legacy flat args for backward compatibility
    /// Port to run the server on
    #[arg(long)]
    port: Option<u16>,

    /// Remote target to connect to (e.g. 192.168.0.2:8000)
    #[arg(long)]
    target: Option<String>,

    /// Optional password for authentication
    #[arg(long)]
    password: Option<String>,

    /// Send a file or folder directly without starting a web panel
    #[arg(long)]
    file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the drift server with web UI
    Serve {
        /// Port to run the server on
        #[arg(long)]
        port: u16,
        /// Remote target to connect to (e.g. 192.168.0.2:8000)
        #[arg(long)]
        target: Option<String>,
        /// Optional password for authentication
        #[arg(long)]
        password: Option<String>,
    },
    /// Send a file or folder to a remote drift server
    Send {
        /// Remote target (e.g. 192.168.0.2:8000)
        #[arg(long)]
        target: String,
        /// File or folder to send
        path: PathBuf,
        /// Optional password for authentication
        #[arg(long)]
        password: Option<String>,
    },
    /// List files on a remote drift server
    Ls {
        /// Remote target (e.g. 192.168.0.2:8000)
        #[arg(long)]
        target: String,
        /// Remote path to list (defaults to root)
        path: Option<String>,
        /// Optional password for authentication
        #[arg(long)]
        password: Option<String>,
    },
    /// Pull a file or folder from a remote drift server
    Pull {
        /// Remote target (e.g. 192.168.0.2:8000)
        #[arg(long)]
        target: String,
        /// Remote path to pull
        remote_path: String,
        /// Local output directory (defaults to current directory)
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Optional password for authentication
        #[arg(long)]
        password: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("drift=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Serve { port, target, password }) => {
            run_server(port, target, password).await
        }
        Some(Commands::Send { target, path, password }) => {
            client::send::send_file(&target, &path, &password).await
        }
        Some(Commands::Ls { target, path, password }) => {
            client::browse::browse_remote(&target, path.as_deref(), &password).await
        }
        Some(Commands::Pull { target, remote_path, output, password }) => {
            client::pull::pull_remote(&target, &remote_path, output.as_deref(), &password).await
        }
        None => {
            // Backward compatibility with flat args
            if let Some(ref file_path) = cli.file {
                let target = cli.target.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("--file requires --target"))?;
                return client::send::send_file(target, file_path, &cli.password).await;
            }

            let port = cli.port
                .ok_or_else(|| anyhow::anyhow!(
                    "--port is required (or use a subcommand: serve, send, ls, pull)"
                ))?;

            run_server(port, cli.target, cli.password).await
        }
    }
}

async fn run_server(port: u16, target: Option<String>, password: Option<String>) -> anyhow::Result<()> {
    let config = AppConfig {
        target: target.clone(),
        password: password.clone(),
        root_dir: std::env::current_dir()?,
        hostname: hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string()),
    };

    let state = Arc::new(server::AppState::new(config.clone()));

    if let Some(ref target) = config.target {
        let target = target.clone();
        let state_clone = state.clone();
        let password = config.password.clone();
        tokio::spawn(async move {
            if let Err(e) = client::connect_to_remote(&target, &password, state_clone).await {
                tracing::error!("Failed to connect to remote: {}", e);
            }
        });
    }

    server::run(state, port).await?;

    Ok(())
}
