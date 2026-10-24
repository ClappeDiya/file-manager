//! Export one sync pair as a versioned JSON file and import it on another
//! computer (WS-C item 45).
//!
//! ```json
//! {"format": "ufop-sync-pair", "version": 1, "exported_at": "…",
//!  "pair": { …SyncPair… },
//!  "connections": [{"side": "source", "connection_id": "…", "name": "NAS", …}]}
//! ```
//!
//! A pair is two local folders, but either may be the mount point of a
//! saved connection (an SFTP or cloud mount). Those connections are listed
//! so the importing computer can say which ones it lacks instead of the
//! pair silently failing on a folder that is not there.

use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, CopyOptions, SyncPair};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FORMAT: &str = "ufop-sync-pair";
pub const VERSION: u32 = 1;

/// A saved connection whose mount point holds one side of the pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRef {
    /// `source` or `dest`.
    pub side: String,
    pub connection_id: String,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub mount_point: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairExport {
    pub format: String,
    pub version: u32,
    pub exported_at: DateTime<Utc>,
    pub pair: SyncPair,
    #[serde(default)]
    pub connections: Vec<ConnectionRef>,
}

/// What an import created, and what the user has to fix before it runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub pair: SyncPair,
    pub warnings: Vec<String>,
}

fn protocol_name(p: &ConnectionProfile) -> String {
    serde_json::to_value(p.protocol).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn under(path: &str, root: &str) -> bool {
    let (p, r) = (Path::new(path), Path::new(root));
    !root.trim().is_empty() && p.starts_with(r)
}

/// Connections mounted at (or above) either folder of `pair`.
/// `mounts`: (connection id, mount point) of every saved mount.
pub fn connection_refs(pair: &SyncPair, mounts: &[(String, String)], profiles: &[ConnectionProfile]) -> Vec<ConnectionRef> {
    let mut out = Vec::new();
    for (side, path) in [("source", &pair.source_path), ("dest", &pair.dest_path)] {
        // The deepest mount point containing the path is the one it is on.
        let best = mounts.iter().filter(|(_, mp)| under(path, mp)).max_by_key(|(_, mp)| mp.len());
        if let Some((cid, mp)) = best {
            let profile = profiles.iter().find(|p| p.id.to_string() == *cid);
            out.push(ConnectionRef {
                side: side.to_string(),
                connection_id: cid.clone(),
                name: profile.map(|p| p.name.clone()).unwrap_or_default(),
                protocol: profile.map(protocol_name).unwrap_or_default(),
                host: profile.map(|p| p.host.clone()).unwrap_or_default(),
                mount_point: mp.clone(),
            });
        }
    }
    out
}

pub fn export(pair: &SyncPair, connections: Vec<ConnectionRef>) -> Result<String, AppError> {
    let doc = PairExport { format: FORMAT.to_string(), version: VERSION, exported_at: Utc::now(), pair: pair.clone(), connections };
    Ok(serde_json::to_string_pretty(&doc)?)
}

/// Read and validate an exported pair.
pub fn parse(json: &str) -> Result<PairExport, AppError> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| AppError::validation(format!("This is not a sync pair export: {e}")))?;
    if value.get("format").and_then(|f| f.as_str()) != Some(FORMAT) {
        return Err(AppError::validation("This file is not a UFOP sync pair export (format \"ufop-sync-pair\")"));
    }
    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version == 0 || version > VERSION as u64 {
        return Err(AppError::validation(format!(
            "Sync pair export version {version} is not supported (this app reads version {VERSION}); update the app"
        )));
    }
    let doc: PairExport = serde_json::from_value(value).map_err(|e| AppError::validation(format!("The sync pair in this file is incomplete: {e}")))?;
    let p = &doc.pair;
    if p.name.trim().is_empty() || p.source_path.trim().is_empty() || p.dest_path.trim().is_empty() {
        return Err(AppError::validation("The exported pair needs a name, a source and a destination"));
    }
    ufop_core::sync::copier::validate_options(&p.copy_options)?;
    ufop_core::sync::executor::validate_run_options(&p.run_options)?;
    ufop_core::sync::filter::validate(&p.filter)?;
    if let Some(own) = &p.option_overrides {
        CopyOptions::validate_override_names(own)?;
    }
    Ok(doc)
}

/// What is missing on this computer for the imported pair to run.
pub fn warnings(doc: &PairExport, profiles: &[ConnectionProfile], path_exists: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    for c in &doc.connections {
        let found = profiles.iter().any(|p| {
            p.id.to_string() == c.connection_id || (!c.name.is_empty() && p.name == c.name && p.host == c.host && protocol_name(p) == c.protocol)
        });
        let side = if c.side == "source" { "source" } else { "destination" };
        if !found {
            let label = if c.name.is_empty() { c.connection_id.clone() } else { format!("\"{}\" ({} {})", c.name, c.protocol, c.host) };
            out.push(format!("The {side} is on connection {label}, which is not set up on this computer: add it and mount it at {}.", c.mount_point));
        }
    }
    for (side, path) in [("source", &doc.pair.source_path), ("destination", &doc.pair.dest_path)] {
        if !path_exists(path) {
            out.push(format!("The {side} folder {path} does not exist on this computer."));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::ConnectionProtocol;

    fn profile(name: &str) -> ConnectionProfile {
        serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4(), "name": name, "protocol": "sftp", "host": "nas.local", "port": 22,
            "username": "me", "credential_ref": null, "remote_path": "/", "created_at": Utc::now(), "last_used": null,
            "bandwidth_limit_bps": 0, "verify_checksums": false
        }))
        .unwrap()
    }

    #[test]
    fn round_trip_and_missing_connections() {
        let nas = profile("NAS");
        assert_eq!(nas.protocol, ConnectionProtocol::Sftp);
        let pair = SyncPair { name: "Photos".into(), source_path: "/home/me/Pictures".into(), dest_path: "/mnt/nas/photos/2026".into(), ..Default::default() };
        let mounts = vec![(nas.id.to_string(), "/mnt/nas".to_string()), (nas.id.to_string(), "/mnt/nas/photos".to_string())];
        let refs = connection_refs(&pair, &mounts, std::slice::from_ref(&nas));
        assert_eq!(refs.len(), 1);
        assert_eq!((refs[0].side.as_str(), refs[0].mount_point.as_str(), refs[0].name.as_str()), ("dest", "/mnt/nas/photos", "NAS"));

        let json = export(&pair, refs).unwrap();
        let doc = parse(&json).unwrap();
        assert_eq!(doc.version, VERSION);
        assert_eq!(doc.pair.name, "Photos");
        // Same profile here (matched by id): only the missing folder is flagged.
        let w = warnings(&doc, std::slice::from_ref(&nas), |p| p.starts_with("/mnt"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("/home/me/Pictures"));
        // Another computer with a same-named profile (different id) is fine too.
        let other = profile("NAS");
        assert!(warnings(&doc, &[other], |_| true).is_empty());
        // No such connection: flagged with its name and mount point.
        let w = warnings(&doc, &[], |_| true);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("\"NAS\" (sftp nas.local)") && w[0].contains("/mnt/nas/photos"), "{w:?}");
    }

    #[test]
    fn rejects_other_files_and_newer_versions() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"format":"ufop-backup-job","version":1}"#).unwrap_err().to_string().contains("not a UFOP sync pair"));
        let pair = SyncPair { name: "x".into(), source_path: "/a".into(), dest_path: "/b".into(), ..Default::default() };
        let mut v: serde_json::Value = serde_json::from_str(&export(&pair, vec![]).unwrap()).unwrap();
        v["version"] = 2.into();
        assert!(parse(&v.to_string()).unwrap_err().to_string().contains("version 2"));
        v["version"] = 1.into();
        v["pair"]["name"] = "".into();
        assert!(parse(&v.to_string()).is_err());
        v["pair"]["name"] = "x".into();
        v["pair"]["copy_options"]["threads"] = 0.into();
        assert!(parse(&v.to_string()).is_err(), "invalid options are refused");
    }

    /// Review: create and update refuse unknown `option_overrides` names, but
    /// an imported file could store them (and they were silently ignored).
    #[test]
    fn rejects_unknown_option_override_names() {
        let pair = SyncPair {
            name: "x".into(),
            source_path: "/a".into(),
            dest_path: "/b".into(),
            option_overrides: Some(vec!["threads".into()]),
            ..Default::default()
        };
        let mut v: serde_json::Value = serde_json::from_str(&export(&pair, vec![]).unwrap()).unwrap();
        assert!(parse(&v.to_string()).is_ok(), "real option names are accepted");
        v["pair"]["option_overrides"] = serde_json::json!(["threads", "no_such_option"]);
        let err = parse(&v.to_string()).unwrap_err().to_string();
        assert!(err.contains("no_such_option"), "{err}");
    }
}
