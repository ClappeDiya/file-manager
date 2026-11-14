//! Native SMB/CIFS destination (no mount needed) through Samba's
//! `smbclient`, with a minimum protocol (e.g. SMB 3.1.1) and optional
//! transport encryption.

use super::{BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use ufop_core::backup::RemoteFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SmbConfig {
    pub server: String,
    pub share: String,
    /// Folder inside the share (`/`-separated).
    pub path: String,
    pub username: String,
    pub domain: String,
    pub port: Option<u16>,
    /// Lowest protocol accepted: `SMB2`, `SMB3`, `SMB3_11` (empty = client default).
    pub min_protocol: String,
    /// Require SMB3 transport encryption.
    pub encrypt: bool,
    /// smbclient executable.
    pub binary: String,
}

impl Default for SmbConfig {
    fn default() -> Self {
        Self { server: String::new(), share: String::new(), path: String::new(), username: String::new(), domain: String::new(), port: None, min_protocol: "SMB3".into(), encrypt: false, binary: "smbclient".into() }
    }
}

pub struct SmbStore {
    cfg: SmbConfig,
    password: Option<String>,
}

/// Private temp file removed on drop.
struct TempPath(std::path::PathBuf);

impl TempPath {
    fn new() -> Result<Self, AppError> {
        let p = ufop_core::session::user_temp_subdir("smb").join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::File::create(&p).map_err(|e| AppError::internal(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self(p))
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Quote for smbclient's command parser.
fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('"', ""))
}

/// Parse one `ls` line: `  name   A   1234  Wed Oct 21 10:00:00 2026`.
pub fn parse_ls_line(line: &str) -> Option<(String, bool, u64)> {
    let t: Vec<&str> = line.split_whitespace().collect();
    if t.len() < 8 {
        return None;
    }
    let n = t.len();
    // Last five tokens: weekday, month, day, time, year; before them: size.
    let size: u64 = t[n - 6].parse().ok()?;
    // Attribute letters (may be absent for plain files on some servers).
    let (attrs, name_end) = if t[n - 7].chars().all(|c| "DAHSRNV".contains(c)) { (t[n - 7], n - 7) } else { ("", n - 6) };
    // Recover the name with its original spacing.
    let first = t.first()?;
    let start = line.find(first)?;
    let last_name_tok = t[name_end - 1];
    let mut end = start;
    for tok in &t[..name_end] {
        end = line[end..].find(tok)? + end + tok.len();
    }
    let _ = last_name_tok;
    let name = line[start..end].to_string();
    if name == "." || name == ".." {
        return None;
    }
    Some((name, attrs.contains('D'), size))
}

impl SmbStore {
    pub fn new(cfg: SmbConfig, password: Option<String>) -> Self {
        Self { cfg, password }
    }

    fn dir(&self) -> String {
        self.cfg.path.trim_matches('/').replace('/', "\\")
    }

    async fn run(&self, commands: &str) -> Result<String, AppError> {
        // Credentials go through an auth file, never the command line.
        let auth = TempPath::new()?;
        let mut body = format!("username = {}\npassword = {}\n", self.cfg.username, self.password.clone().unwrap_or_default());
        if !self.cfg.domain.is_empty() {
            body.push_str(&format!("domain = {}\n", self.cfg.domain));
        }
        std::fs::write(auth.path(), body).map_err(|e| AppError::internal(e.to_string()))?;
        let mut cmd = tokio::process::Command::new(&self.cfg.binary);
        cmd.arg(format!("//{}/{}", self.cfg.server, self.cfg.share)).arg("-A").arg(auth.path()).arg("-c").arg(commands);
        if let Some(p) = self.cfg.port {
            cmd.arg("-p").arg(p.to_string());
        }
        if !self.cfg.min_protocol.is_empty() {
            cmd.arg(format!("--option=client min protocol={}", self.cfg.min_protocol));
        }
        if self.cfg.encrypt {
            cmd.arg("--client-protection=encrypt");
        }
        let out = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output().await.map_err(|e| AppError::connection(format!("Could not run {}: {e}", self.cfg.binary), "Install the Samba client (smbclient)."))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let all = format!("{stdout}\n{stderr}");
        if let Some(l) = all.lines().find(|l| l.contains("NT_STATUS_") && !l.contains("NT_STATUS_OBJECT_NAME_COLLISION")) {
            return Err(AppError::connection(format!("SMB: {}", l.trim()), "Check the server, share, credentials and protocol setting."));
        }
        if !out.status.success() && !all.contains("NT_STATUS_OBJECT_NAME_COLLISION") {
            return Err(AppError::connection(format!("smbclient failed: {}", stderr.trim()), "Check the SMB settings."));
        }
        Ok(stdout)
    }

    fn cd(&self) -> String {
        let d = self.dir();
        if d.is_empty() { String::new() } else { format!("cd {}; ", q(&d)) }
    }
}

impl ObjectStore for SmbStore {
    fn describe(&self) -> String {
        format!("smb://{}/{}/{}", self.cfg.server, self.cfg.share, self.cfg.path.trim_matches('/'))
    }
    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let out = self.run(&format!("{}ls", self.cd())).await?;
            Ok(out.lines().filter_map(parse_ls_line).filter(|(_, dir, _)| !dir).map(|(name, _, size)| RemoteFile { name, size }).collect())
        })
    }
    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let tmp = TempPath::new()?;
            std::fs::write(tmp.path(), &data).map_err(|e| AppError::internal(e.to_string()))?;
            self.run(&format!("{}put {} {}", self.cd(), q(&tmp.path().to_string_lossy()), q(name))).await?;
            Ok(())
        })
    }
    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let tmp = TempPath::new()?;
            self.run(&format!("{}get {} {}", self.cd(), q(name), q(&tmp.path().to_string_lossy()))).await?;
            std::fs::read(tmp.path()).map_err(|e| AppError::connection(format!("SMB download of {name}: {e}"), "Retry."))
        })
    }
    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            self.run(&format!("{}del {}", self.cd(), q(name))).await?;
            Ok(())
        })
    }
    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            let mut acc = String::new();
            for part in self.dir().split('\\').filter(|p| !p.is_empty()) {
                acc = if acc.is_empty() { part.to_string() } else { format!("{acc}\\{part}") };
                self.run(&format!("mkdir {}", q(&acc))).await?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_lines() {
        assert_eq!(parse_ls_line("  ufop-repo.json                      A      312  Wed Oct 21 10:00:00 2026"), Some(("ufop-repo.json".into(), false, 312)));
        assert_eq!(parse_ls_line("  my file.txt                         A        5  Wed Oct 21 10:00:00 2026"), Some(("my file.txt".into(), false, 5)));
        assert_eq!(parse_ls_line("  sub                                 D        0  Wed Oct 21 10:00:00 2026").map(|x| x.1), Some(true));
        assert_eq!(parse_ls_line("  .                                   D        0  Wed Oct 21 10:00:00 2026"), None);
        assert_eq!(parse_ls_line("\t\t46410756 blocks of size 4096. 12345 blocks available"), None);
    }
}
