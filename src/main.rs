mod config;
mod client;
mod crypto;
mod fileops;
mod frontend;
mod protocol;
mod server;

use clap::Parser;
use config::AppConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "drift", about = "Encrypted file transfer over WebSocket")]
struct Cli {
    /// Port to run the server on (not needed with --file)
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("drift=info".parse()?))
        .init();

    let cli = Cli::parse();

    // Direct file send mode: drift --target host:port --file path
    if let Some(ref file_path) = cli.file {
        let target = cli.target.as_ref()
            .ok_or_else(|| anyhow::anyhow!("--file requires --target"))?;

        return client::send::send_file(target, file_path, &cli.password).await;
    }

    // Server mode: requires --port
    let port = cli.port
        .ok_or_else(|| anyhow::anyhow!("--port is required (or use --file --target for direct send)"))?;

    let config = AppConfig {
        target: cli.target.clone(),
        password: cli.password.clone(),
        root_dir: std::env::current_dir()?,
        hostname: hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string()),
    };

    let state = Arc::new(server::AppState::new(config.clone()));

    // If --target is provided, spawn client connection
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

    // Start the server
    server::run(state, port).await?;

    Ok(())
}
