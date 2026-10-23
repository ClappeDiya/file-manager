//! `ufop profile` — named profiles (API server, tokens, output format) kept in
//! the CLI config file. Pick one per run with `--profile <name>` /
//! `UFOP_PROFILE`, or make it the default with `ufop profile use <name>`.

use crate::config::{valid_profile_name, CliConfig, Profile, DEFAULT_PROFILE};
use crate::output;
use crate::OutputFormat;
use clap::Subcommand;
use serde::Serialize;

#[derive(Subcommand, Debug)]
pub enum ProfileCommands {
    /// List profiles (the active one is marked)
    List,
    /// Show a profile's settings (tokens are masked); the active one by default
    Show { name: Option<String> },
    /// Make a profile the default for later runs ("default" = the top-level settings)
    Use { name: String },
    /// Create a profile, optionally copying another's settings
    Create {
        name: String,
        /// Copy settings (including tokens) from this profile
        #[arg(long)]
        from: Option<String>,
        /// API server URL
        #[arg(long = "api")]
        api: Option<String>,
        /// `ufop server` URL
        #[arg(long)]
        server_url: Option<String>,
        /// Default output format (human, json, yaml, csv)
        #[arg(long = "default-format")]
        default_format: Option<String>,
    },
    /// Delete a profile and its stored tokens
    Remove { name: String },
}

#[derive(Serialize)]
struct ProfileView {
    name: String,
    active: bool,
    api_url: Option<String>,
    token: Option<String>,
    server_url: Option<String>,
    server_token: Option<String>,
    default_format: Option<String>,
}

fn mask(s: &Option<String>) -> Option<String> {
    s.as_ref().map(|t| if t.len() > 12 { format!("{}…{}", &t[..8], &t[t.len() - 4..]) } else { "****".into() })
}

fn view(name: &str, p: &Profile, active: bool) -> ProfileView {
    ProfileView {
        name: name.into(),
        active,
        api_url: p.api_url.clone(),
        token: mask(&p.token),
        server_url: p.server_url.clone(),
        server_token: mask(&p.server_token),
        default_format: p.default_format.clone(),
    }
}

/// Every profile name, "default" first.
fn names(c: &CliConfig) -> Vec<String> {
    let mut v = vec![DEFAULT_PROFILE.to_string()];
    v.extend(c.profiles.keys().filter(|k| *k != DEFAULT_PROFILE).cloned());
    if !c.has_profile(c.profile_name()) {
        v.push(c.profile_name().to_string());
    }
    v
}

pub fn parse_format(s: &str) -> Result<OutputFormat, String> {
    <OutputFormat as clap::ValueEnum>::from_str(s, true).map_err(|_| format!("unknown output format '{s}' (use human, json, yaml or csv)"))
}

pub async fn execute(cmd: ProfileCommands, format: &OutputFormat) -> Result<u8, Box<dyn std::error::Error>> {
    let mut c = CliConfig::load();
    match cmd {
        ProfileCommands::List => {
            let active = c.profile_name().to_string();
            let views: Vec<ProfileView> = names(&c).iter().map(|n| view(n, &c.profile(n).unwrap_or_default(), *n == active)).collect();
            let rows: Vec<Vec<String>> = views
                .iter()
                .map(|v| vec![if v.active { format!("* {}", v.name) } else { format!("  {}", v.name) }, v.api_url.clone().unwrap_or_default(), v.server_url.clone().unwrap_or_default(), v.default_format.clone().unwrap_or_default()])
                .collect();
            match format {
                // The `*` marker is for people; CSV gets the `active` column.
                OutputFormat::Csv => output::print_data(format, &views),
                _ => output::print_table(format, &["PROFILE", "API URL", "SERVER URL", "FORMAT"], &rows, &views),
            }
        }
        ProfileCommands::Show { name } => {
            let name = name.unwrap_or_else(|| c.profile_name().to_string());
            let p = c.profile(&name).ok_or_else(|| format!("no profile named '{name}'"))?;
            output::print_data(format, &view(&name, &p, name == c.profile_name()));
        }
        ProfileCommands::Use { name } => {
            if !c.has_profile(&name) {
                return Err(format!("no profile named '{name}' (create it with `ufop profile create {name}`)").into());
            }
            c.current_profile = (name != DEFAULT_PROFILE).then(|| name.clone());
            c.save()?;
            output::print_success(format, &format!("Now using profile '{name}'"));
        }
        ProfileCommands::Create { name, from, api, server_url, default_format } => {
            if !valid_profile_name(&name) || name == DEFAULT_PROFILE {
                return Err(format!("'{name}' is not a usable profile name (letters, digits, '-', '_', '.')").into());
            }
            if c.profile_name() == name {
                return Err(format!("profile '{name}' is selected for this run; create it without --profile / UFOP_PROFILE").into());
            }
            if c.profiles.contains_key(&name) {
                return Err(format!("profile '{name}' already exists").into());
            }
            if let Some(f) = &default_format {
                parse_format(f)?;
            }
            let mut p = match &from {
                Some(src) => c.profile(src).ok_or_else(|| format!("no profile named '{src}'"))?,
                None => Profile::default(),
            };
            p.api_url = api.or(p.api_url);
            p.server_url = server_url.or(p.server_url);
            p.default_format = default_format.or(p.default_format);
            c.profiles.insert(name.clone(), p);
            c.save()?;
            output::print_success(format, &format!("Created profile '{name}'"));
        }
        ProfileCommands::Remove { name } => {
            if name == DEFAULT_PROFILE {
                return Err("the default profile cannot be removed".into());
            }
            if c.profile_name() == name {
                return Err(format!("profile '{name}' is in use; switch with `ufop profile use default` or --profile first").into());
            }
            if c.profiles.remove(&name).is_none() {
                return Err(format!("no profile named '{name}'").into());
            }
            if c.current_profile.as_deref() == Some(name.as_str()) {
                c.current_profile = None;
            }
            c.save()?;
            output::print_success(format, &format!("Removed profile '{name}'"));
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_tokens_and_parses_formats() {
        assert_eq!(mask(&Some("ufop_pat_abcdefghijklmnop".into())).unwrap(), "ufop_pat…mnop");
        assert_eq!(mask(&Some("short".into())).unwrap(), "****");
        assert!(matches!(parse_format("JSON"), Ok(OutputFormat::Json)));
        assert!(parse_format("xml").is_err());
    }
}
