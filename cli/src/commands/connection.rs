//! Connection management commands: list, add, test, remove.

use crate::config::{CliConfig, SavedConnection};
use crate::output;
use crate::ConnectionCommands;
use crate::OutputFormat;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct ConnectionInfo {
    name: String,
    uri: String,
    protocol: String,
    created_at: String,
}

pub async fn execute(
    cmd: ConnectionCommands,
    format: &OutputFormat,
    _dry_run: bool,
) -> Result<u8, Box<dyn std::error::Error>> {
    match cmd {
        ConnectionCommands::List { protocol } => {
            let config = CliConfig::load();
            let connections: Vec<ConnectionInfo> = config
                .connections
                .iter()
                .filter(|c| {
                    protocol
                        .as_ref()
                        .map_or(true, |p| c.protocol.eq_ignore_ascii_case(p))
                })
                .map(|c| ConnectionInfo {
                    name: c.name.clone(),
                    uri: c.uri.clone(),
                    protocol: c.protocol.clone(),
                    created_at: c.created_at.clone(),
                })
                .collect();

            let rows: Vec<Vec<String>> = connections
                .iter()
                .map(|c| vec![c.name.clone(), c.protocol.clone(), c.uri.clone(), c.created_at.clone()])
                .collect();

            match format {
                OutputFormat::Human => {
                    println!("\n  Saved Connections ({} total):\n", connections.len());
                    output::print_table(
                        format,
                        &["NAME", "PROTOCOL", "URI", "CREATED"],
                        &rows,
                        &connections,
                    );
                    println!();
                }
                _ => output::print_data(format, &connections),
            }
            Ok(0)
        }

        ConnectionCommands::Add { name, uri, save_password: _ } => {
            let mut config = CliConfig::load();

            // Parse protocol from URI
            let protocol = if uri.starts_with("sftp://") {
                "sftp"
            } else if uri.starts_with("ftp://") || uri.starts_with("ftps://") {
                "ftp"
            } else if uri.starts_with("webdav://") || uri.starts_with("https://") {
                "webdav"
            } else if uri.starts_with("s3://") {
                "s3"
            } else if uri.starts_with("smb://") {
                "smb"
            } else if uri.starts_with("nfs://") {
                "nfs"
            } else {
                "local"
            };

            // Check for duplicate name
            if config.connections.iter().any(|c| c.name == name) {
                output::print_error(format, &format!("Connection '{name}' already exists"));
                return Ok(2);
            }

            config.connections.push(SavedConnection {
                name: name.clone(),
                uri: uri.clone(),
                protocol: protocol.to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
            });
            config.save()?;

            output::print_success(
                format,
                &format!("Connection '{name}' added ({protocol}): {uri}"),
            );
            Ok(0)
        }

        ConnectionCommands::Test { target } => {
            let config = CliConfig::load();
            let uri = config
                .connections
                .iter()
                .find(|c| c.name == target)
                .map(|c| c.uri.clone())
                .unwrap_or_else(|| target.clone());

            match format {
                OutputFormat::Human => {
                    println!("  Testing connection to {uri}...");
                }
                _ => {}
            }

            // Attempt basic connectivity check
            if uri.starts_with("http://") || uri.starts_with("https://") {
                let client = reqwest::Client::new();
                match client
                    .head(&uri)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(resp) => {
                        let status = resp.status();
                        if status.is_success() || status.as_u16() == 401 || status.as_u16() == 405 {
                            output::print_success(
                                format,
                                &format!("Connection successful (HTTP {status})"),
                            );
                            return Ok(0);
                        } else {
                            output::print_error(
                                format,
                                &format!("Connection returned HTTP {status}"),
                            );
                            return Ok(2);
                        }
                    }
                    Err(e) => {
                        output::print_error(format, &format!("Connection failed: {e}"));
                        return Ok(2);
                    }
                }
            }

            // For non-HTTP protocols, report as untestable from CLI alone
            output::print_warning(
                format,
                &format!("Connection test for protocol not available in CLI. Use the desktop app for full testing."),
            );
            Ok(0)
        }

        ConnectionCommands::Remove { name } => {
            let mut config = CliConfig::load();
            let before = config.connections.len();
            config.connections.retain(|c| c.name != name);
            if config.connections.len() == before {
                output::print_error(format, &format!("Connection '{name}' not found"));
                return Ok(2);
            }
            config.save()?;
            output::print_success(format, &format!("Connection '{name}' removed"));
            Ok(0)
        }
    }
}
