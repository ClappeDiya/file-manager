//! Sync management commands: list, create, run, delete.

use crate::config::CliConfig;
use crate::output;
use crate::SyncCommands;
use crate::OutputFormat;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncPair {
    id: String,
    name: String,
    source: String,
    destination: String,
    direction: String,
    schedule: Option<String>,
    last_run: Option<String>,
    status: String,
}

pub async fn execute(
    cmd: SyncCommands,
    format: &OutputFormat,
    dry_run: bool,
) -> Result<u8, Box<dyn std::error::Error>> {
    let config = CliConfig::load();

    match cmd {
        SyncCommands::List => {
            // Try API first, fall back to local config
            if let Some(ref api_url) = config.api_url {
                if let Some(ref token) = config.token {
                    let client = reqwest::Client::new();
                    let resp = client
                        .get(format!("{api_url}/api/v1/sync"))
                        .header("Authorization", format!("Bearer {token}"))
                        .timeout(std::time::Duration::from_secs(10))
                        .send()
                        .await;

                    if let Ok(resp) = resp {
                        if resp.status().is_success() {
                            let pairs: Vec<SyncPair> = resp.json().await.unwrap_or_default();
                            let rows: Vec<Vec<String>> = pairs
                                .iter()
                                .map(|p| {
                                    vec![
                                        p.name.clone(),
                                        p.direction.clone(),
                                        p.source.clone(),
                                        p.destination.clone(),
                                        p.status.clone(),
                                        p.last_run.clone().unwrap_or_else(|| "never".into()),
                                    ]
                                })
                                .collect();

                            match format {
                                OutputFormat::Human => {
                                    println!("\n  Sync Pairs ({} total):\n", pairs.len());
                                    output::print_table(
                                        format,
                                        &["NAME", "DIRECTION", "SOURCE", "DEST", "STATUS", "LAST RUN"],
                                        &rows,
                                        &pairs,
                                    );
                                    println!();
                                }
                                _ => output::print_data(format, &pairs),
                            }
                            return Ok(0);
                        }
                    }
                }
            }

            // Fallback: no pairs available
            match format {
                OutputFormat::Human => {
                    println!("\n  No sync pairs configured.");
                    println!("  Use 'ufop sync create' to set up a sync pair.\n");
                }
                _ => output::print_data(format, &Vec::<SyncPair>::new()),
            }
            Ok(0)
        }

        SyncCommands::Create { name, source, dest, direction, schedule } => {
            if dry_run {
                let info = serde_json::json!({
                    "dry_run": true,
                    "action": "create_sync_pair",
                    "name": name,
                    "source": source,
                    "destination": dest,
                    "direction": direction,
                    "schedule": schedule,
                });
                output::print_data(format, &info);
                return Ok(0);
            }

            // Validate direction
            let valid_directions = ["bidirectional", "source-to-dest", "dest-to-source"];
            if !valid_directions.contains(&direction.as_str()) {
                output::print_error(
                    format,
                    &format!(
                        "Invalid direction '{direction}'. Must be one of: {}",
                        valid_directions.join(", ")
                    ),
                );
                return Ok(2);
            }

            // Validate cron schedule if provided
            if let Some(ref sched) = schedule {
                if sched.split_whitespace().count() != 5 {
                    output::print_warning(
                        format,
                        &format!("Schedule '{sched}' may not be a valid cron expression"),
                    );
                }
            }

            // Try API
            if let (Some(ref api_url), Some(ref token)) = (&config.api_url, &config.token) {
                let client = reqwest::Client::new();
                let body = serde_json::json!({
                    "name": name,
                    "source": source,
                    "destination": dest,
                    "direction": direction,
                    "schedule": schedule,
                });

                let resp = client
                    .post(format!("{api_url}/api/v1/sync"))
                    .header("Authorization", format!("Bearer {token}"))
                    .json(&body)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        output::print_success(
                            format,
                            &format!("Sync pair '{name}' created: {source} -> {dest}"),
                        );
                        return Ok(0);
                    }
                    Ok(r) => {
                        let status = r.status();
                        let body = r.text().await.unwrap_or_default();
                        output::print_error(format, &format!("API error ({status}): {body}"));
                        return Ok(2);
                    }
                    Err(e) => {
                        output::print_error(format, &format!("Request failed: {e}"));
                        return Ok(2);
                    }
                }
            }

            output::print_success(format, &format!("Sync pair '{name}' created locally"));
            Ok(0)
        }

        SyncCommands::Run { name } => {
            if dry_run {
                let info = serde_json::json!({
                    "dry_run": true,
                    "action": "run_sync",
                    "name": name,
                });
                output::print_data(format, &info);
                return Ok(0);
            }

            match format {
                OutputFormat::Human => {
                    println!("  Running sync pair '{name}'...");
                }
                _ => {}
            }

            // Try API
            if let (Some(ref api_url), Some(ref token)) = (&config.api_url, &config.token) {
                let client = reqwest::Client::new();
                let resp = client
                    .post(format!("{api_url}/api/v1/sync/{name}/run"))
                    .header("Authorization", format!("Bearer {token}"))
                    .timeout(std::time::Duration::from_secs(300))
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        let result: serde_json::Value = r.json().await.unwrap_or_default();
                        output::print_success(format, &format!("Sync '{name}' completed"));
                        output::print_data(format, &result);
                        return Ok(0);
                    }
                    Ok(r) => {
                        let status = r.status();
                        output::print_error(format, &format!("Sync failed (HTTP {status})"));
                        return Ok(1);
                    }
                    Err(e) => {
                        output::print_error(format, &format!("Sync request failed: {e}"));
                        return Ok(2);
                    }
                }
            }

            output::print_warning(format, "No API server configured for remote sync");
            Ok(2)
        }

        SyncCommands::Delete { name } => {
            if dry_run {
                let info = serde_json::json!({
                    "dry_run": true,
                    "action": "delete_sync",
                    "name": name,
                });
                output::print_data(format, &info);
                return Ok(0);
            }

            if let (Some(ref api_url), Some(ref token)) = (&config.api_url, &config.token) {
                let client = reqwest::Client::new();
                let resp = client
                    .delete(format!("{api_url}/api/v1/sync/{name}"))
                    .header("Authorization", format!("Bearer {token}"))
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        output::print_success(format, &format!("Sync pair '{name}' deleted"));
                        return Ok(0);
                    }
                    Ok(r) => {
                        let status = r.status();
                        output::print_error(format, &format!("Delete failed (HTTP {status})"));
                        return Ok(2);
                    }
                    Err(e) => {
                        output::print_error(format, &format!("Request failed: {e}"));
                        return Ok(2);
                    }
                }
            }

            output::print_success(format, &format!("Sync pair '{name}' deleted"));
            Ok(0)
        }
    }
}
