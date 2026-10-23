//! `ufop completions <shell>` and `ufop man`: shell completion scripts and
//! man pages generated from the CLI's own command tree
//! (`ufop_core::cli_gen`).

use std::path::Path;
use ufop_core::cli_gen::{self, Shell};

/// Shells `ufop completions` accepts.
pub const SHELLS: [&str; 5] = ["bash", "zsh", "fish", "powershell", "elvish"];

/// The completion script for `shell`.
pub fn script(cmd: &clap::Command, shell: &str) -> Result<String, String> {
    let sh = Shell::parse(shell).ok_or_else(|| format!("unknown shell '{shell}' (bash, zsh, fish, powershell, elvish)"))?;
    Ok(cli_gen::completions(cmd, sh))
}

/// Write every page to `out` (returns the file names), or return the page
/// of `path` (`["backup", "run"]`; empty = the top page).
pub fn man(cmd: &clap::Command, out: Option<&Path>, path: &[String]) -> Result<Vec<(String, String)>, String> {
    let pages = cli_gen::man_pages(cmd);
    match out {
        Some(dir) => {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            for (name, body) in &pages {
                std::fs::write(dir.join(name), body).map_err(|e| format!("cannot write {}: {e}", dir.join(name).display()))?;
            }
            Ok(pages.into_iter().map(|(n, _)| (n, String::new())).collect())
        }
        None => {
            let mut want = vec![cmd.get_name().to_string()];
            want.extend(path.iter().cloned());
            let name = format!("{}.1", want.join("-"));
            pages.into_iter().find(|(n, _)| *n == name).map(|p| vec![p]).ok_or_else(|| format!("no command '{}'", want.join(" ")))
        }
    }
}
