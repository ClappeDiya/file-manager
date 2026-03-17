//! Login command - authenticate with UFOP server or store API key.

use crate::config::CliConfig;
use crate::output;
use crate::OutputFormat;

pub async fn execute(
    token: Option<String>,
    server: Option<String>,
    format: &OutputFormat,
) -> Result<u8, Box<dyn std::error::Error>> {
    let mut config = CliConfig::load();

    // Set server URL if provided
    if let Some(url) = server {
        config.api_url = Some(url.clone());
        output::print_success(format, &format!("API server set to {url}"));
    }

    // Handle token-based auth
    if let Some(tok) = token {
        // Validate token format
        if tok.len() < 8 {
            output::print_error(format, "Token appears invalid (too short)");
            return Ok(3); // Auth error
        }

        // If we have a server URL, attempt to validate the token
        if let Some(ref api_url) = config.api_url {
            let client = reqwest::Client::new();
            let resp = client
                .get(format!("{api_url}/api/v1/health"))
                .header("Authorization", format!("Bearer {tok}"))
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await;

            match resp {
                Ok(r) if r.status().is_success() => {
                    config.token = Some(tok);
                    config.save()?;
                    output::print_success(format, "Authenticated successfully");
                    return Ok(0);
                }
                Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
                    output::print_error(format, "Authentication failed: invalid token");
                    return Ok(3);
                }
                Ok(_) | Err(_) => {
                    // Server might not be reachable, store token anyway
                    config.token = Some(tok);
                    config.save()?;
                    output::print_warning(
                        format,
                        "Could not validate token against server. Token saved locally.",
                    );
                    return Ok(0);
                }
            }
        } else {
            // No server configured, just store the token
            config.token = Some(tok);
            config.save()?;
            output::print_success(
                format,
                "Token saved. Use --server to configure the API endpoint.",
            );
            return Ok(0);
        }
    }

    // Interactive login prompt (human mode only)
    match format {
        OutputFormat::Human => {
            if config.api_url.is_none() && server.is_none() {
                output::print_error(
                    format,
                    "No API server configured. Use: ufop login --server <url> --token <token>",
                );
                return Ok(3);
            }
            output::print_success(format, "Login configuration updated");
        }
        _ => {
            if token.is_none() {
                output::print_error(format, "Token required for non-interactive login");
                return Ok(3);
            }
        }
    }

    config.save()?;
    Ok(0)
}
