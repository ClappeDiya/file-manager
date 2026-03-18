//! Status command - show server and platform information.

use crate::config::CliConfig;
use crate::output;
use crate::OutputFormat;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct StatusInfo {
    version: String,
    platform: String,
    arch: String,
    api_configured: bool,
    api_url: Option<String>,
    authenticated: bool,
    connections_count: usize,
    server_status: Option<String>,
}

pub async fn execute(
    format: &OutputFormat,
) -> Result<u8, Box<dyn std::error::Error>> {
    let config = CliConfig::load();

    let mut status = StatusInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        api_configured: config.api_url.is_some(),
        api_url: config.api_url.clone(),
        authenticated: config.token.is_some(),
        connections_count: config.connections.len(),
        server_status: None,
    };

    // Check server health if configured
    if let Some(ref api_url) = config.api_url {
        let client = reqwest::Client::new();
        let mut req = client
            .get(format!("{api_url}/api/v1/health"))
            .timeout(std::time::Duration::from_secs(5));

        if let Some(ref token) = config.token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                status.server_status = Some("healthy".into());
            }
            Ok(resp) => {
                status.server_status = Some(format!("error (HTTP {})", resp.status()));
            }
            Err(_) => {
                status.server_status = Some("unreachable".into());
            }
        }
    }

    match format {
        OutputFormat::Human => {
            println!();
            println!("  UFOP CLI v{}", status.version);
            println!("  Platform: {} ({})", status.platform, status.arch);
            println!();
            println!("  API Server: {}", status.api_url.as_deref().unwrap_or("not configured"));
            println!("  Authenticated: {}", if status.authenticated { "yes" } else { "no" });
            println!("  Saved Connections: {}", status.connections_count);

            if let Some(ref server_status) = status.server_status {
                let color = if server_status == "healthy" { "\x1b[32m" } else { "\x1b[31m" };
                println!("  Server Status: {color}{server_status}\x1b[0m");
            }
            println!();
        }
        _ => output::print_data(format, &status),
    }

    Ok(0)
}
