//! rclone crypt remotes: what they wrap, and offline name encoding.
//!
//! A crypt remote (`type = crypt`) encrypts names and contents on top of
//! another remote (its `remote` option). Browsing it through rclone shows
//! decrypted names; [`info`] tells the UI which remote holds the encrypted
//! data, and [`decode`] wraps `rclone cryptdecode [--reverse]` to translate
//! names without touching the remote.

use super::resolver::Rclone;
use crate::core::error::AppError;
use serde::Serialize;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(60);
/// Names translated in one call.
pub const MAX_NAMES: usize = 1000;

/// What a remote is, for the browser banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CryptInfo {
    /// The remote name (`secret`).
    pub remote: String,
    pub is_crypt: bool,
    /// The crypt remote's `remote` option as configured (`gdrive:vault`, `/data`).
    pub underlying: Option<String>,
    /// `underlying` as something the rclone connector can open
    /// (a local folder becomes `:local:/data`).
    pub underlying_remote: Option<String>,
    /// `standard`, `obfuscate` or `off`.
    pub filename_encryption: Option<String>,
    pub directory_name_encryption: Option<bool>,
}

/// `secret:` / `secret:sub/dir` → `secret`; on-the-fly (`:crypt,…:`) and
/// local paths have no configured name.
pub fn remote_name(remote: &str) -> Option<String> {
    let r = remote.trim();
    if r.starts_with(':') || is_local_path(r) {
        return None;
    }
    let name = r.split_once(':').map(|(n, _)| n).unwrap_or(r);
    super::config::validate_name(name).ok()
}

/// `/data`, `C:\data`, `\\server\share`, `./x` (not `remote:path`).
fn is_local_path(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with('/') || s.starts_with('\\') || s.starts_with('.') || s.starts_with('~') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'\\' || b[2] == b'/')) || !s.contains(':')
}

/// A crypt `remote` option as a remote the connector can open.
pub fn browse_target(underlying: &str) -> String {
    let u = underlying.trim();
    if is_local_path(u) {
        format!(":local:{u}")
    } else {
        u.to_string()
    }
}

/// Describe `remote` from its config section.
pub fn info_from(remote: &str, section: Option<&std::collections::BTreeMap<String, String>>) -> CryptInfo {
    let name = remote_name(remote).unwrap_or_else(|| remote.trim().to_string());
    let crypt = section.filter(|s| s.get("type").map(String::as_str) == Some("crypt"));
    let underlying = crypt.and_then(|s| s.get("remote")).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    CryptInfo {
        remote: name,
        is_crypt: crypt.is_some(),
        underlying_remote: underlying.as_deref().map(browse_target),
        underlying,
        filename_encryption: crypt.map(|s| s.get("filename_encryption").cloned().unwrap_or_else(|| "standard".into())),
        directory_name_encryption: crypt.map(|s| s.get("directory_name_encryption").map(|v| v != "false").unwrap_or(true)),
    }
}

/// Whether `remote` is a crypt remote, and what it wraps.
pub async fn info(rc: &Rclone, remote: &str) -> Result<CryptInfo, AppError> {
    let Some(name) = remote_name(remote) else { return Ok(info_from(remote, None)) };
    let all = super::config::dump(rc).await?;
    Ok(info_from(remote, all.get(&name)))
}

/// One translated name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CryptName {
    pub input: String,
    pub output: Option<String>,
    pub error: Option<String>,
}

/// `rclone cryptdecode` output (`input \t output`, one line per name) in
/// the order of `names`.
pub fn parse_decode(out: &str, names: &[String]) -> Vec<CryptName> {
    let mut found: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for line in out.lines() {
        if let Some((i, o)) = line.split_once(" \t ") {
            found.insert(i, o);
        }
    }
    names
        .iter()
        .map(|n| match found.get(n.as_str()) {
            Some(o) if o.trim() == "Failed to decrypt" => CryptName { input: n.clone(), output: None, error: Some("Not a name encrypted with this remote's password".into()) },
            Some(o) => CryptName { input: n.clone(), output: Some(o.to_string()), error: None },
            None => CryptName { input: n.clone(), output: None, error: Some("rclone returned no result for this name".into()) },
        })
        .collect()
}

/// Decrypt (`reverse` = false) or encrypt (`reverse` = true) `names` with
/// crypt remote `remote`'s keys. Nothing on the remote is read.
pub async fn decode(rc: &Rclone, remote: &str, names: &[String], reverse: bool) -> Result<Vec<CryptName>, AppError> {
    let name = remote_name(remote).ok_or_else(|| AppError::validation("Choose a configured crypt remote"))?;
    let names: Vec<String> = names.iter().map(|n| n.trim_end_matches(['\r', '\n']).to_string()).filter(|n| !n.trim().is_empty()).collect();
    if names.is_empty() {
        return Ok(Vec::new());
    }
    if names.len() > MAX_NAMES {
        return Err(AppError::validation(format!("Translate at most {MAX_NAMES} names at a time")));
    }
    if names.iter().any(|n| n.contains(['\n', '\t'])) {
        return Err(AppError::validation("Names cannot contain tabs or line breaks"));
    }
    let mut args: Vec<String> = vec!["cryptdecode".into()];
    if reverse {
        args.push("--reverse".into());
    }
    args.push(format!("{name}:"));
    // Names after `--` are never read as flags.
    args.push("--".into());
    args.extend(names.iter().cloned());
    let out = rc.run(&args, TIMEOUT).await?;
    Ok(parse_decode(&out, &names))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn names_targets_and_info() {
        assert_eq!(remote_name("secret:").as_deref(), Some("secret"));
        assert_eq!(remote_name("secret:sub/dir").as_deref(), Some("secret"));
        assert_eq!(remote_name("secret").as_deref(), None, "a bare word is a local path to rclone");
        assert_eq!(remote_name(":crypt,remote=/x:"), None);
        assert_eq!(remote_name("/data"), None);
        assert_eq!(remote_name("C:\\data"), None);
        assert_eq!(browse_target("/srv/vault"), ":local:/srv/vault");
        assert_eq!(browse_target("C:\\vault"), ":local:C:\\vault");
        assert_eq!(browse_target("gdrive:vault"), "gdrive:vault");
        assert_eq!(browse_target("vault"), ":local:vault");

        let s: BTreeMap<String, String> = [("type", "crypt"), ("remote", "gdrive:vault"), ("filename_encryption", "off"), ("directory_name_encryption", "false")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let i = info_from("sec:docs", Some(&s));
        assert!(i.is_crypt);
        assert_eq!((i.remote.as_str(), i.underlying.as_deref(), i.underlying_remote.as_deref()), ("sec", Some("gdrive:vault"), Some("gdrive:vault")));
        assert_eq!((i.filename_encryption.as_deref(), i.directory_name_encryption), (Some("off"), Some(false)));
        let plain: BTreeMap<String, String> = [("type", "drive")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let i = info_from("gd:", Some(&plain));
        assert!(!i.is_crypt && i.underlying.is_none() && i.filename_encryption.is_none());
    }

    #[test]
    fn parses_cryptdecode_output_in_order() {
        let names = vec!["b".to_string(), "a b/c".to_string(), "zz".to_string(), "gone".to_string()];
        let out = "a b/c \t x/y\nzz \t Failed to decrypt\nb \t q\n";
        let r = parse_decode(out, &names);
        assert_eq!(r[0].output.as_deref(), Some("q"));
        assert_eq!(r[1].output.as_deref(), Some("x/y"));
        assert!(r[2].output.is_none() && r[2].error.as_deref().unwrap().contains("Not a name"));
        assert!(r[3].error.is_some());
    }

    /// Real rclone crypt remote in a temp config: encode, decode, a name
    /// that is not ours, info, and a non-crypt remote.
    #[tokio::test]
    async fn crypt_with_real_rclone() {
        if !std::process::Command::new("rclone").arg("version").output().is_ok_and(|o| o.status.success()) {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let rc = Rclone::new("rclone", Some(d.path().join("rclone.conf").to_string_lossy().into_owned()));
        let under = d.path().join("under");
        std::fs::create_dir_all(&under).unwrap();
        let p = |kv: &[(&str, &str)]| kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        super::super::config::create(&rc, "sec", "crypt", &p(&[("remote", under.to_str().unwrap()), ("password", "hunter2")]), super::super::config::ObscureMode::Obscure).await.unwrap();
        super::super::config::create(&rc, "loc", "local", &BTreeMap::new(), Default::default()).await.unwrap();

        let plain = vec!["dir".to_string(), "a b/c d.txt".to_string(), "-dash".to_string()];
        let enc = decode(&rc, "sec:", &plain, true).await.unwrap();
        assert!(enc.iter().all(|n| n.error.is_none()), "{enc:?}");
        let encrypted: Vec<String> = enc.iter().map(|n| n.output.clone().unwrap()).collect();
        assert!(encrypted.iter().all(|e| !e.contains("dash") && !e.contains(' ')));
        assert_eq!(encrypted[1].matches('/').count(), 1, "path segments are encrypted one by one");
        let mut back_in = encrypted.clone();
        back_in.push("notencrypted".into());
        let back = decode(&rc, "sec:sub", &back_in, false).await.unwrap();
        assert_eq!(back.iter().take(3).map(|n| n.output.clone().unwrap()).collect::<Vec<_>>(), plain);
        assert!(back[3].output.is_none() && back[3].error.is_some());

        // The same names on the underlying folder after a real upload.
        let src = d.path().join("hello.txt");
        std::fs::write(&src, "hi").unwrap();
        rc.run(&["copyto".into(), src.to_string_lossy().into_owned(), "sec:dir/hello.txt".into()], TIMEOUT).await.unwrap();
        let on_disk: Vec<String> = std::fs::read_dir(&under).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(on_disk, vec![encrypted[0].clone()]);

        let i = info(&rc, "sec:").await.unwrap();
        assert!(i.is_crypt);
        assert_eq!(i.underlying.as_deref(), Some(under.to_str().unwrap()));
        assert_eq!(i.underlying_remote, Some(format!(":local:{}", under.display())));
        assert!(!info(&rc, "loc:").await.unwrap().is_crypt);
        assert!(!info(&rc, ":local:/tmp").await.unwrap().is_crypt);
        // cryptdecode refuses non-crypt remotes.
        assert!(decode(&rc, "loc:", &plain, false).await.unwrap_err().user_message().contains("crypt"));
        assert!(decode(&rc, "/tmp", &plain, false).await.is_err());
        assert!(decode(&rc, "sec:", &[], false).await.unwrap().is_empty());
    }
}
