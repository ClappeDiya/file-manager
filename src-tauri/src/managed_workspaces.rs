//! Install console grants as saved desktop connections; keep secrets in the secure store.
use crate::{
    core::{
        error::AppError,
        types::{ConnectionProfile, ConnectionProtocol},
    },
    storage::Repository,
};
use std::collections::{BTreeMap, BTreeSet};
use ufop_core::agent::WorkspaceDelivery;
use uuid::Uuid;
const OWNED: &str = "fleet.workspace-connection-ids";
const SNAPSHOT: &str = "fleet.workspace-snapshot";

pub async fn owned(repo: &Repository) -> Result<BTreeSet<Uuid>, AppError> {
    match repo.get_config(OWNED).await? {
        Some(raw) => serde_json::from_str(&raw).map_err(|_| AppError::validation("The managed connection ownership record is damaged; synchronize the console before editing connections")),
        None => Ok(BTreeSet::new()),
    }
}
pub async fn ensure_editable(repo: &Repository, id: Uuid) -> Result<(), AppError> {
    if owned(repo).await?.contains(&id) {
        return Err(AppError::state(
            "This connection is managed by your organization",
            "Change the connector or workspace grant in the admin console.",
        ));
    }
    Ok(())
}
fn profile(
    c: &ufop_core::agent::ManagedConnection,
    workspace: &str,
) -> Result<ConnectionProfile, AppError> {
    Ok(ConnectionProfile {
        id: Uuid::parse_str(&c.id)
            .map_err(|_| AppError::validation("Invalid managed connection id"))?,
        name: format!("{workspace} · {}", c.name),
        protocol: ConnectionProtocol::from_str_lossy(&c.protocol),
        host: c.host.clone(),
        port: c.port,
        username: (!c.username.is_empty()).then(|| c.username.clone()),
        credential_ref: c.credential_id.clone(),
        remote_path: c.remote_path.clone(),
        created_at: chrono::Utc::now(),
        last_used: None,
        group_id: None,
        bandwidth_limit_bps: 0,
        conflict_policy: None,
        retry_policy: None,
        verify_checksums: true,
        checksum_algorithm: None,
        proxy_type: None,
        proxy_host: None,
        proxy_port: None,
        proxy_username: None,
        proxy_password: None,
        default_local_dir: None,
        default_remote_dir: None,
        charset: None,
        default_file_mode: None,
        default_dir_mode: None,
        ftp_use_mlsd: None,
        ftp_force_passive_ip: None,
        ftp_post_login_commands: None,
        symlink_policy: None,
        extras: crate::core::types::ConnectionExtras {
            managed_by: Some(workspace.to_string()),
            ..Default::default()
        },
    })
}
/// Validate the whole snapshot before mutations. Record ownership intent before writes,
/// so interruption/retry can safely clean up any partially installed profiles.
pub async fn apply(repo: &Repository, workspaces: Vec<WorkspaceDelivery>) -> Result<(), AppError> {
    if workspaces.len() > 500 {
        return Err(AppError::validation("Too many workspaces"));
    }
    let mut next = BTreeMap::new();
    for w in &workspaces {
        w.validate().map_err(AppError::validation)?;
        for c in &w.connections {
            let p = profile(c, &w.name)?;
            if let Some(previous) = next.get(&p.id) {
                let previous: &ConnectionProfile = previous;
                if previous.host != p.host
                    || previous.protocol != p.protocol
                    || previous.port != p.port
                    || previous.username != p.username
                    || previous.remote_path != p.remote_path
                    || previous.credential_ref != p.credential_ref
                {
                    return Err(AppError::validation(
                        "Conflicting definitions of the same managed connection",
                    ));
                }
            } else {
                next.insert(p.id, p);
            }
        }
    }
    let old = owned(repo).await?;
    for id in next.keys() {
        if !old.contains(id) && repo.get_connection(*id).await?.is_some() {
            return Err(AppError::validation(
                "A managed connection id conflicts with a personal connection",
            ));
        }
    }
    let wanted: BTreeSet<Uuid> = next.keys().copied().collect();
    repo.set_config(
        OWNED,
        &serde_json::to_string(&old.union(&wanted).copied().collect::<BTreeSet<_>>())?,
    )
    .await?;
    for p in next.values() {
        repo.save_connection(p).await?;
    }
    for id in old.difference(&wanted) {
        repo.delete_connection(*id).await?;
    }
    repo.set_config(SNAPSHOT, &serde_json::to_string(&workspaces)?)
        .await?;
    repo.set_config(OWNED, &serde_json::to_string(&wanted)?)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ufop_core::agent::ManagedConnection;
    fn delivery() -> WorkspaceDelivery {
        WorkspaceDelivery {
            id: Uuid::new_v4().to_string(),
            name: "Design team".into(),
            version: 1,
            connections: vec![ManagedConnection {
                id: Uuid::new_v4().to_string(),
                name: "Assets".into(),
                protocol: "sftp".into(),
                host: "files.example.org".into(),
                remote_path: "/assets".into(),
                credential_id: Some("central:crd_test".into()),
                ..Default::default()
            }],
        }
    }
    #[tokio::test]
    async fn persists_actual_profiles_updates_revokes_and_preserves_personal_connections() {
        let repo = Repository::open_in_memory().await.unwrap();
        let mut w = delivery();
        let id = Uuid::parse_str(&w.connections[0].id).unwrap();
        let mut personal = profile(&w.connections[0], "Personal").unwrap();
        personal.id = Uuid::new_v4();
        repo.save_connection(&personal).await.unwrap();
        apply(&repo, vec![w.clone()]).await.unwrap();
        let p = repo.get_connection(id).await.unwrap().unwrap();
        assert_eq!(p.host, "files.example.org");
        assert_eq!(p.credential_ref.as_deref(), Some("central:crd_test"));
        assert!(ensure_editable(&repo, id).await.is_err());
        w.connections[0].host = "new.example.org".into();
        w.version = 2;
        apply(&repo, vec![w]).await.unwrap();
        assert_eq!(
            repo.get_connection(id).await.unwrap().unwrap().host,
            "new.example.org"
        );
        apply(&repo, vec![]).await.unwrap();
        assert!(repo.get_connection(id).await.unwrap().is_none());
        assert!(repo.get_connection(personal.id).await.unwrap().is_some());
        assert!(owned(&repo).await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn invalid_snapshot_and_id_collision_leave_existing_profiles_intact() {
        let repo = Repository::open_in_memory().await.unwrap();
        let mut w = delivery();
        let p = profile(&w.connections[0], "Personal").unwrap();
        repo.save_connection(&p).await.unwrap();
        assert!(apply(&repo, vec![w.clone()]).await.is_err());
        assert!(owned(&repo).await.unwrap().is_empty());
        w.connections[0].id = Uuid::new_v4().to_string();
        w.connections[0].credential_id = Some("plain-secret".into());
        assert!(apply(&repo, vec![w]).await.is_err());
        assert_eq!(repo.list_connections().await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn central_secret_reference_survives_reopen_and_is_resolved_from_the_vault() {
        use crate::connectors::ConnectionManager;
        use crate::security::{credential_store::FileVault, CredentialStore};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("desktop.sqlite");
        let vault_path = dir.path().join("credentials.vault");
        let w = delivery();
        let id = Uuid::parse_str(&w.connections[0].id).unwrap();
        {
            let repo = Repository::new(crate::storage::pool::DbPool::open(&database).unwrap())
                .await
                .unwrap();
            let vault = Arc::new(FileVault::open(&vault_path, None).unwrap());
            let creds = Arc::new(CredentialStore::with_file_vault(vault));
            use ufop_core::agent::AgentHost;
            let backups = crate::backup_engine::BackupManager::with_kdf(repo.pool().clone(), creds.clone(), None, None, dir.path().join("backup-dbs"), ufop_core::backup::crypto::KdfParams::insecure_fast());
            let settings = crate::security::settings_store::SettingsStore::open(repo.pool().clone(), &creds, Default::default()).await.unwrap();
            assert!(settings.is_encrypted(), "the portable vault protects the settings encryption key");
            backups.set_settings_store(settings.clone());
            let host = crate::fleet_agent::BackupHost::new(backups, settings, creds.clone(), "desktop");
            host.save_token(Some("regression-agent-token".into())).await.unwrap();
            host.store_credential("central:crd_test".into(), Some("regression-secret-bytes".into())).await.unwrap();
            apply(&repo, vec![w]).await.unwrap();
            let manager = ConnectionManager::new(repo, creds);
            assert_eq!(
                manager.get_password(id).await.unwrap().as_deref(),
                Some("regression-secret-bytes")
            );
            assert!(manager.delete_connection(id).await.is_err());
        }
        let repo = Repository::new(crate::storage::pool::DbPool::open(&database).unwrap())
            .await
            .unwrap();
        let creds = Arc::new(CredentialStore::with_file_vault(Arc::new(
            FileVault::open(&vault_path, None).unwrap(),
        )));
        let manager = ConnectionManager::new(repo.clone(), creds);
        assert_eq!(
            manager.get_password(id).await.unwrap().as_deref(),
            Some("regression-secret-bytes")
        );
        assert!(!String::from_utf8_lossy(&std::fs::read(&database).unwrap()).contains("regression-secret-bytes"));
        assert!(!String::from_utf8_lossy(&std::fs::read(&database).unwrap()).contains("regression-agent-token"));
        assert!(
            !String::from_utf8_lossy(&std::fs::read(&vault_path).unwrap())
                .contains("regression-secret-bytes")
        );
        apply(&repo, vec![]).await.unwrap();
        assert!(manager.get_connection(id).await.unwrap().is_none());
    }
}
