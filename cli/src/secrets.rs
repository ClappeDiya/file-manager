//! Secret providers for CLI values written as `$name` (or containing
//! `${name}`): `env://[PREFIX]`, `file:///path` (JSON object or `.env`
//! lines) and `pass://[folder]` (Linux `pass`). Providers are tried in the
//! order given. The desktop app and server also support Vault, AWS, Google
//! and Azure; this covers what a script or cron job needs without a network.

use std::collections::HashMap;

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '#' | '@')
}

fn lookup(provider: &str, name: &str) -> Result<Option<String>, String> {
    if let Some(prefix) = provider.strip_prefix("env://") {
        return Ok(std::env::var(format!("{prefix}{name}")).ok());
    }
    if let Some(path) = provider.strip_prefix("file://") {
        let text = std::fs::read_to_string(path).map_err(|e| format!("secret file {path}: {e}"))?;
        let t = text.trim_start();
        if t.starts_with('{') {
            let v: serde_json::Value = serde_json::from_str(t).map_err(|e| format!("secret file {path}: {e}"))?;
            return Ok(v.get(name).map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())));
        }
        for l in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            if let Some((k, v)) = l.strip_prefix("export ").unwrap_or(l).split_once('=') {
                if k.trim() == name {
                    let v = v.trim();
                    let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).or_else(|| v.strip_prefix('\'').and_then(|x| x.strip_suffix('\''))).unwrap_or(v);
                    return Ok(Some(v.to_string()));
                }
            }
        }
        return Ok(None);
    }
    if let Some(folder) = provider.strip_prefix("pass://") {
        let entry = if folder.is_empty() { name.to_string() } else { format!("{}/{name}", folder.trim_end_matches('/')) };
        let out = std::process::Command::new(std::env::var("UFOP_PASS_EXECUTABLE").unwrap_or_else(|_| "pass".into()))
            .args(["show", &entry])
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("could not run pass: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return if err.contains("not in the password store") { Ok(None) } else { Err(format!("pass show {entry}: {}", err.trim())) };
        }
        return Ok(String::from_utf8_lossy(&out.stdout).lines().next().map(String::from));
    }
    Err(format!("unsupported secret provider '{provider}' (use env://, file:// or pass://)"))
}

fn resolve_name(providers: &[String], name: &str) -> Result<String, String> {
    if providers.is_empty() {
        return Err(format!("'${name}' needs a --secret-provider"));
    }
    let mut errors = Vec::new();
    for p in providers {
        match lookup(p, name) {
            Ok(Some(v)) => return Ok(v),
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
    }
    Err(if errors.is_empty() { format!("secret '{name}' was not found") } else { format!("secret '{name}': {}", errors.join("; ")) })
}

/// Resolve placeholders in `value`; values without any are returned as is.
pub fn resolve(value: &str, providers: &[String]) -> Result<String, String> {
    if let Some(n) = value.strip_prefix('$').filter(|n| !n.is_empty() && !n.starts_with(['$', '{']) && n.chars().all(name_char)) {
        return resolve_name(providers, n);
    }
    if !value.contains("${") && !value.contains("$$") {
        return Ok(value.to_string());
    }
    let mut cache: HashMap<String, String> = HashMap::new();
    let mut out = String::new();
    let mut rest = value;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(r) = after.strip_prefix('$') {
            out.push('$');
            rest = r;
        } else if let Some((n, r)) = after.strip_prefix('{').and_then(|a| a.split_once('}')).filter(|(n, _)| !n.is_empty() && n.chars().all(name_char)) {
            if !cache.contains_key(n) {
                cache.insert(n.to_string(), resolve_name(providers, n)?);
            }
            out.push_str(&cache[n]);
            rest = r;
        } else {
            out.push('$');
            rest = after;
        }
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_and_file_providers() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("s.env");
        std::fs::write(&f, "# x\nDB='from file'\n").unwrap();
        std::env::set_var("UFOPCLI_TEST_TOKEN", "from-env");
        let ps = vec!["env://UFOPCLI_TEST_".to_string(), format!("file://{}", f.display())];
        assert_eq!(resolve("$TOKEN", &ps).unwrap(), "from-env");
        assert_eq!(resolve("$DB", &ps).unwrap(), "from file");
        assert_eq!(resolve("a ${DB} b $$5", &ps).unwrap(), "a from file b $5");
        assert_eq!(resolve("plain", &[]).unwrap(), "plain");
        assert!(resolve("$MISSING", &ps).unwrap_err().contains("not found"));
        assert!(resolve("$X", &[]).unwrap_err().contains("--secret-provider"));
        assert!(resolve("$X", &["vault://x".into()]).unwrap_err().contains("unsupported"));
    }
}
