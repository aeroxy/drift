use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::messages::{ControlMessage, FileEntry};

use super::perform_client_handshake;
use super::send::{send_encrypted_control, recv_encrypted_control};

/// Connect to a remote drift server and list files at the given path.
pub async fn browse_remote(
    target: &str,
    path: Option<&str>,
    password: &Option<String>,
    allow_insecure_tls: bool,
) -> anyhow::Result<()> {
    let (ws_stream, _) = super::open_ws(target, allow_insecure_tls).await?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let (crypto, fp) = perform_client_handshake(&mut ws_write, &mut ws_read, password).await?;
    tracing::info!("Encrypted connection established (fingerprint: {})", fp);

    let browse_path = path.unwrap_or(".").to_string();
    send_encrypted_control(&crypto, &mut ws_write, &ControlMessage::BrowseRequest {
        path: browse_path,
    }).await?;

    let response = recv_encrypted_control(&crypto, &mut ws_read).await?;

    match response {
        ControlMessage::BrowseResponse { hostname, cwd, entries } => {
            println!("{}:{}", hostname, cwd);
            if entries.is_empty() {
                println!("  (empty)");
            } else {
                print_entries(&entries);
            }
        }
        ControlMessage::Error { message } => {
            anyhow::bail!("{}", message);
        }
        other => {
            anyhow::bail!("Unexpected response: {:?}", other);
        }
    }

    let _ = ws_write.send(Message::Close(None)).await;
    Ok(())
}

fn print_entries(entries: &[FileEntry]) {
    // Find max size string width for alignment
    let size_strings: Vec<String> = entries.iter().map(|e| format_size(e.size)).collect();
    let max_size_width = size_strings.iter().map(|s| s.len()).max().unwrap_or(0);

    for (entry, size_str) in entries.iter().zip(size_strings.iter()) {
        #[cfg(unix)]
        let perms = format_permissions(entry.permissions, entry.is_dir);
        #[cfg(not(unix))]
        let perms = if entry.is_dir { "d---------" } else { "----------" }.to_string();

        let timestamp = format_timestamp(entry.modified);
        let name = if entry.is_dir {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        };

        println!("{}  {:>width$}  {}  {}",
            perms, size_str, timestamp, name,
            width = max_size_width,
        );
    }
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}G", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}M", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}

#[cfg(unix)]
fn format_permissions(mode: u32, is_dir: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        s.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    s
}

fn format_timestamp(unix_secs: u64) -> String {
    // Convert unix timestamp to YYYY-MM-DD HH:MM without chrono dependency
    let secs = unix_secs as i64;
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;

    // Days since 1970-01-01 to year/month/day (civil calendar)
    let (year, month, day) = days_to_date(days);

    format!("{:04}-{:02}-{:02} {:02}:{:02}", year, month, day, hours, minutes)
}

fn days_to_date(days_since_epoch: i64) -> (i64, u32, u32) {
    // Algorithm from Howard Hinnant's chrono-compatible date library
    let z = days_since_epoch + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
