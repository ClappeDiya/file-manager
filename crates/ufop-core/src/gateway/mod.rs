//! File gateway: an on-premises agent that makes local folders reachable
//! through the organization's own `ufop-server`, without opening an inbound
//! port. The agent connects out (HTTPS long polling), takes jobs the server
//! queued for its gateway name, and streams results back; the server never
//! stores the data it relays.
//!
//! - Several agents may serve one gateway name (high availability): each
//!   job goes to whichever node polls first, so a stopped node simply stops
//!   taking work.
//! - Paths are `/<root name>/<relative path>`; `/` lists the roots. They are
//!   confined to the agent's roots after resolving symlinks, and dot-files are
//!   neither listed nor served.
//! - Uploads are refused unless the agent was started writable; they are
//!   written under a hidden temporary name and renamed into place.
//!
//! This module holds the wire protocol (shared by server and agent); the
//! agent loop is behind the `gateway` feature.

use serde::{Deserialize, Serialize};

#[cfg(feature = "gateway")]
mod agent;
#[cfg(feature = "gateway")]
pub use agent::{run, AgentConfig};

/// Longest a poll waits for a job before the server answers 204.
pub const POLL_WAIT_SECS: u64 = 25;
/// A node that has not polled for this long is offline.
pub const NODE_TIMEOUT_SECS: i64 = 60;
/// Reply header: `ok` or `error` (the body is then an [`ErrorBody`]).
pub const STATUS_HEADER: &str = "x-ufop-gateway-status";
/// Reply header on downloads: the file size.
pub const SIZE_HEADER: &str = "x-ufop-gateway-size";

/// What a polling agent tells the server about itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hello {
    /// Gateway name clients use (`office-nas`); nodes sharing it form a pool.
    pub gateway: String,
    /// This node's id (stable per agent process).
    pub node: String,
    pub host: String,
    /// Root names served (`/<name>/…`).
    pub roots: Vec<String>,
    pub writable: bool,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    List { path: String },
    Download { path: String },
    /// The agent fetches the data from `/api/v1/gateway/jobs/{id}/body`.
    Upload { path: String, size: Option<u64> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub id: String,
    #[serde(flatten)]
    pub op: Op,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// RFC 3339.
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Listing {
    pub path: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorBody {
    pub message: String,
    /// `not_found`, `forbidden`, `invalid` or `failed`.
    pub kind: String,
}

/// Valid gateway and node names: 1–64 of `A-Z a-z 0-9 - _ .`.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && !s.starts_with('.')
}

/// `/<root>/<rel>` → (`root`, `rel`) with `.`/`..`/dot-file components refused.
pub fn split_path(path: &str) -> Result<(String, String), String> {
    let mut segs = Vec::new();
    for s in path.split(['/', '\\']).filter(|s| !s.is_empty()) {
        if s == "." || s == ".." || s.starts_with('.') || s.contains('\0') {
            return Err(format!("{path}: hidden or relative components are not allowed"));
        }
        segs.push(s);
    }
    match segs.split_first() {
        None => Ok((String::new(), String::new())),
        Some((root, rest)) => Ok((root.to_string(), rest.join("/"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_and_paths() {
        let j = Job { id: "1".into(), op: Op::Upload { path: "/docs/a.txt".into(), size: Some(3) } };
        let v = serde_json::to_value(&j).unwrap();
        assert_eq!(v, serde_json::json!({"id": "1", "op": "upload", "path": "/docs/a.txt", "size": 3}));
        assert_eq!(serde_json::from_value::<Job>(v).unwrap(), j);
        assert_eq!(split_path("/docs/a b/c.txt").unwrap(), ("docs".into(), "a b/c.txt".into()));
        assert_eq!(split_path("/").unwrap(), (String::new(), String::new()));
        for bad in ["/docs/../x", "/docs/.ssh/id", "/docs/./x"] {
            assert!(split_path(bad).is_err(), "{bad}");
        }
        assert!(valid_name("office-nas_1.eu") && !valid_name("") && !valid_name("a b") && !valid_name(".x") && !valid_name(&"x".repeat(65)));
    }
}
