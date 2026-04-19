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
#[command(name = "drift", about = "Encrypted file transfer over WebSocket", version = concat!("v", env!("CARGO_PKG_VERSION")))]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    // Legacy flat args for backward compatibility
    /// Port to run the server on
    #[arg(long)]
    port: Option<u16>,

    /// Remote target to connect to (e.g. 192.168.0.2:8000 or wss://example.com)
    #[arg(long)]
    target: Option<String>,

    /// Optional password for authentication
    #[arg(long)]
    password: Option<String>,

    /// Send a file or folder directly without starting a web panel
    #[arg(long)]
    file: Option<PathBuf>,

    /// Accept self-signed or invalid TLS certificates (use with wss:// targets)
    #[arg(long)]
    allow_insecure_tls: bool,

    /// Disable the web UI and REST API — expose only the /ws endpoint
    #[arg(long)]
    disable_ui: bool,

    /// Run the server in the background (logs appended to ./drift.log in the current directory)
    #[arg(long)]
    daemon: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a file or folder to a remote drift server
    Send {
        /// Remote target (e.g. 192.168.0.2:8000 or wss://example.com)
        #[arg(long)]
        target: String,
        /// File or folder to send
        path: PathBuf,
        /// Optional password for authentication
        #[arg(long)]
        password: Option<String>,
        /// Accept self-signed or invalid TLS certificates
        #[arg(long)]
        allow_insecure_tls: bool,
    },
    /// List files on a remote drift server
    Ls {
        /// Remote target (e.g. 192.168.0.2:8000 or wss://example.com)
        #[arg(long)]
        target: String,
        /// Remote path to list (defaults to root)
        path: Option<String>,
        /// Optional password for authentication
        #[arg(long)]
        password: Option<String>,
        /// Accept self-signed or invalid TLS certificates
        #[arg(long)]
        allow_insecure_tls: bool,
    },
    /// Pull a file or folder from a remote drift server
    Pull {
        /// Remote target (e.g. 192.168.0.2:8000 or wss://example.com)
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
        /// Accept self-signed or invalid TLS certificates
        #[arg(long)]
        allow_insecure_tls: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("drift=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Send { target, path, password, allow_insecure_tls }) => {
            client::send::send_file(&target, &path, &password, allow_insecure_tls).await
        }
        Some(Commands::Ls { target, path, password, allow_insecure_tls }) => {
            client::browse::browse_remote(&target, path.as_deref(), &password, allow_insecure_tls).await
        }
        Some(Commands::Pull { target, remote_path, output, password, allow_insecure_tls }) => {
            client::pull::pull_remote(&target, &remote_path, output.as_deref(), &password, allow_insecure_tls).await
        }
        None => {
            if cli.daemon { return start_daemon(); }

            if let Some(ref file_path) = cli.file {
                let target = cli.target.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("--file requires --target"))?;
                return client::send::send_file(target, file_path, &cli.password, cli.allow_insecure_tls).await;
            }

            run_server(cli.port, cli.target, cli.password, cli.allow_insecure_tls, cli.disable_ui).await
        }
    }
}

fn start_daemon() -> anyhow::Result<()> {
    let log_path = std::env::current_dir()?.join("drift.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true).append(true).open(&log_path)?;

    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().skip(1)
        .filter(|a| a != "--daemon")
        .collect();

    #[cfg(unix)]
    let child = {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(&exe)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(log_file.try_clone()?)
            .stderr(log_file)
            .process_group(0)
            .spawn()?
    };
    #[cfg(not(unix))]
    let child = std::process::Command::new(&exe)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .spawn()?;

    println!("drift daemon started (PID: {})", child.id());
    println!("Logs: {}", log_path.display());
    Ok(())
}

async fn run_server(
    port: Option<u16>,
    target: Option<String>,
    password: Option<String>,
    allow_insecure_tls: bool,
    disable_ui: bool,
) -> anyhow::Result<()> {
    let config = AppConfig {
        target: target.clone(),
        password: password.clone(),
        root_dir: std::env::current_dir()?,
        hostname: hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string()),
        allow_insecure_tls,
        disable_ui,
    };

    let state = Arc::new(server::AppState::new(config.clone()));

    if let Some(ref target) = config.target {
        let target = target.clone();
        let state_clone = state.clone();
        let password = config.password.clone();
        tokio::spawn(async move {
            if let Err(e) = client::connect_to_remote(&target, &password, allow_insecure_tls, state_clone).await {
                tracing::error!("Failed to connect to remote: {}", e);
            }
        });
    }

    server::run(state, port).await?;

    Ok(())
}
