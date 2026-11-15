//! Saved connections granted to this desktop by its console. Secrets travel separately.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceDelivery {
    pub id: String,
    pub name: String,
    pub version: u64,
    pub connections: Vec<ManagedConnection>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ManagedConnection {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: String,
    pub remote_path: String,
    pub credential_id: Option<String>,
}
impl WorkspaceDelivery {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 100
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            || self.name.trim().is_empty()
            || self.name.len() > 480
            || self.version == 0
            || self.connections.len() > 500
        {
            return Err("Invalid workspace identity, revision or connection count".into());
        }
        for c in &self.connections {
            if uuid::Uuid::parse_str(&c.id).is_err()
                || c.name.trim().is_empty()
                || c.name.len() > 480
                || ![
                    "sftp",
                    "ftp",
                    "ftps",
                    "webdav",
                    "smb",
                    "nfs",
                    "s3",
                    "azure_blob",
                    "adls_gen2",
                    "azure_files",
                    "b2",
                ]
                .contains(&c.protocol.as_str())
                || c.host.is_empty()
                || c.host.len() > 253
                || !c
                    .host
                    .chars()
                    .all(|h| h.is_ascii_alphanumeric() || ".:[]-".contains(h))
                || c.port == Some(0)
                || c.username.len() > 800
                || c.remote_path.is_empty()
                || c.remote_path.len() > 8000
            {
                return Err(format!("Invalid managed connection '{}'", c.name));
            }
            if let Some(id) = &c.credential_id {
                if !id.starts_with("central:") || super::policy::central_credential_id(id).is_none()
                {
                    return Err(
                        "A workspace credential must use a central credential reference".into(),
                    );
                }
            }
        }
        Ok(())
    }
}
