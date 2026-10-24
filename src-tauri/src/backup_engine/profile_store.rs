//! Storage of notification profiles on the backup manager: the config table
//! holds the local profiles, the credential store their secrets, and the
//! console delivers read-only ones. The profile types and the pure
//! resolution rules live in `notify_hub::profiles`.

use crate::backup_engine::notify::profiles::{resolve, NotificationProfile, ProfileDeleteOutcome, PROFILES_KEY};
use crate::backup_engine::notify::{Channel, Notification};
use crate::backup_engine::templates::MANAGED_PREFIX;
use crate::backup_engine::BackupJob;
use crate::core::error::AppError;
use chrono::Utc;

impl super::BackupManager {
    async fn local_profiles(&self) -> Result<Vec<NotificationProfile>, AppError> {
        Ok(self.config_get(PROFILES_KEY).await?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    /// Local profiles, then the organization's (read-only), by name.
    pub async fn list_notification_profiles(&self) -> Result<Vec<NotificationProfile>, AppError> {
        let mut v = self.local_profiles().await?;
        v.extend(self.inner.managed_profiles.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned());
        v.sort_by_key(|p| (p.managed, p.name.to_lowercase()));
        Ok(v)
    }

    pub async fn get_notification_profile(&self, id: &str) -> Result<NotificationProfile, AppError> {
        self.list_notification_profiles().await?.into_iter().find(|p| p.id == id).ok_or_else(|| AppError::not_found(format!("Notification profile {id} not found")))
    }

    /// Move secret header values typed into a channel to the credential
    /// store (the channel keeps only their credential ids).
    pub async fn stash_notification_secrets(&self, n: &mut Notification) -> Result<(), AppError> {
        if let Channel::Webhook { headers, .. } = &mut n.channel {
            for h in headers.iter_mut().filter(|h| h.secret && !h.value.is_empty()) {
                if h.credential_id.trim().is_empty() {
                    h.credential_id = uuid::Uuid::new_v4().to_string();
                }
                self.set_destination_secret(&h.credential_id, &h.value).await?;
                h.value.clear();
            }
        }
        Ok(())
    }

    /// Secret HTTP-report header values to the credential store.
    pub async fn stash_report_secrets(&self, r: &mut crate::backup_engine::hooks::ReportConfig) -> Result<(), AppError> {
        for h in r.headers.iter_mut().filter(|h| h.secret && !h.value.is_empty()) {
            if h.credential_id.trim().is_empty() {
                h.credential_id = uuid::Uuid::new_v4().to_string();
            }
            self.set_destination_secret(&h.credential_id, &h.value).await?;
            h.value.clear();
        }
        Ok(())
    }

    /// HTTP-report settings with secret header values filled in (for sending only).
    pub async fn prepare_reports(&self, r: &crate::backup_engine::hooks::ReportConfig) -> crate::backup_engine::hooks::ReportConfig {
        let mut r = r.clone();
        for h in r.headers.iter_mut().filter(|h| h.secret && h.value.is_empty()) {
            h.value = self.destination_secret(&h.credential_id).await.ok().flatten().unwrap_or_default();
        }
        r
    }

    /// The channel with secret header values filled in, and its secret.
    pub async fn prepare_notification(&self, mut n: Notification) -> (Notification, Option<String>) {
        if let Channel::Webhook { headers, .. } = &mut n.channel {
            for h in headers.iter_mut().filter(|h| h.secret && h.value.is_empty()) {
                h.value = self.destination_secret(&h.credential_id).await.ok().flatten().unwrap_or_default();
            }
        }
        let secret = self.destination_secret(&n.credential_id).await.ok().flatten();
        (n, secret)
    }

    /// Create (empty id) or update a local profile.
    pub async fn save_notification_profile(&self, mut p: NotificationProfile) -> Result<NotificationProfile, AppError> {
        if p.managed || p.id.starts_with(MANAGED_PREFIX) {
            return Err(AppError::PermissionDenied { message: "Changing organization notification profiles is not allowed here; the console manages them".into(), advice: "Create your own profile to change it.".into() });
        }
        p.name = p.name.trim().to_string();
        p.validate()?;
        let mut list = self.local_profiles().await?;
        if list.iter().any(|x| x.id != p.id && x.name.eq_ignore_ascii_case(&p.name)) {
            return Err(AppError::validation(format!("A notification profile named '{}' already exists", p.name)));
        }
        let now = Utc::now().to_rfc3339();
        match list.iter().find(|x| x.id == p.id && !p.id.is_empty()) {
            Some(old) => p.created_at = old.created_at.clone(),
            None => {
                if p.id.is_empty() {
                    p.id = uuid::Uuid::new_v4().to_string();
                }
                p.created_at = now.clone();
            }
        }
        if p.notification.credential_id.trim().is_empty() {
            p.notification.credential_id = uuid::Uuid::new_v4().to_string();
        }
        if let Some(secret) = p.secret.take() {
            self.set_destination_secret(&p.notification.credential_id, &secret).await?;
        }
        self.stash_notification_secrets(&mut p.notification).await?;
        p.updated_at = now;
        match list.iter_mut().find(|x| x.id == p.id) {
            Some(x) => *x = p.clone(),
            None => list.push(p.clone()),
        }
        self.config_set(PROFILES_KEY, serde_json::to_string(&list)?).await?;
        Ok(p)
    }

    /// Delete a local profile. Referenced by jobs or templates: refused
    /// unless `force`, which removes those references (with warnings).
    pub async fn delete_notification_profile(&self, id: &str, force: bool) -> Result<ProfileDeleteOutcome, AppError> {
        if id.starts_with(MANAGED_PREFIX) {
            return Err(AppError::PermissionDenied { message: "Deleting organization notification profiles is not allowed here; the console manages them".into(), advice: "Ask your administrator to remove it.".into() });
        }
        let mut list = self.local_profiles().await?;
        let Some(profile) = list.iter().find(|p| p.id == id).cloned() else {
            return Err(AppError::not_found(format!("Notification profile {id} not found")));
        };
        let refers = |ids: &[String]| ids.iter().any(|x| x == id);
        let jobs: Vec<BackupJob> = self.list_jobs().await.into_iter().filter(|j| refers(&j.notification_profile_ids)).collect();
        let templates: Vec<_> = self.list_templates().await?.into_iter().filter(|t| !t.managed && t.settings.notification_profile_ids.as_deref().is_some_and(refers)).collect();
        let mut users: Vec<String> = jobs.iter().map(|j| format!("job '{}'", j.name)).collect();
        users.extend(templates.iter().map(|t| format!("template '{}'", t.name)));
        if !users.is_empty() && !force {
            return Err(AppError::state(format!("The notification profile is used by {}", users.join(", ")), "Remove it there first, or delete with force to remove those references."));
        }
        let mut out = ProfileDeleteOutcome::default();
        for mut j in jobs {
            j.notification_profile_ids.retain(|x| x != id);
            self.persist(&j).await?;
            self.inner.jobs.write().await.insert(j.id.clone(), j.clone());
            out.warnings.push(format!("Job '{}' no longer sends to '{}'", j.name, profile.name));
            out.removed_from.push(format!("job '{}'", j.name));
        }
        for mut t in templates {
            if let Some(ids) = t.settings.notification_profile_ids.as_mut() {
                ids.retain(|x| x != id);
            }
            out.warnings.push(format!("Template '{}' no longer sends to '{}'", t.name, profile.name));
            out.removed_from.push(format!("template '{}'", t.name));
            self.save_template(t).await?;
        }
        for w in &out.warnings {
            tracing::warn!("{w}");
        }
        list.retain(|p| p.id != id);
        self.config_set(PROFILES_KEY, serde_json::to_string(&list)?).await?;
        for c in profile.notification.credential_ids() {
            let _ = self.set_destination_secret(&c, "").await;
        }
        Ok(out)
    }

    /// Send a test message through a saved profile.
    pub async fn test_notification_profile(&self, id: &str) -> Result<(), AppError> {
        let p = self.get_notification_profile(id).await?;
        self.test_notification(p.effective()).await
    }

    /// Replace the console's profiles; returns the ones that were refused.
    pub fn set_managed_notification_profiles(&self, list: Option<&[ufop_core::agent::ManagedNotificationProfile]>) -> Vec<String> {
        let mut ok = Vec::new();
        let mut problems = Vec::new();
        for m in list.unwrap_or_default() {
            match NotificationProfile::from_managed(m) {
                Ok(p) => ok.push(p),
                Err(e) => problems.push(e.user_message()),
            }
        }
        *self.inner.managed_profiles.lock().unwrap_or_else(|e| e.into_inner()) = ok;
        problems
    }

    /// The channels a (resolved) job sends to, with warnings for missing profiles.
    pub async fn job_notifications(&self, job: &BackupJob) -> (Vec<Notification>, Vec<String>) {
        let profiles = if job.notification_profile_ids.is_empty() { Vec::new() } else { self.list_notification_profiles().await.unwrap_or_default() };
        resolve(&job.notifications, &job.notification_profile_ids, &profiles)
    }

    /// What a channel would send for a sample run (`result`: success, warning or error).
    pub fn preview_notification(&self, n: &Notification, result: &str) -> crate::notify_hub::Preview {
        let vars = crate::notify_hub::profiles::sample_vars(result);
        let body = format!(
            "Backup of Documents {result}: 1284 files examined, 70 MiB uploaded\nInfo: Scanning 2 sources\n{}",
            if result == "error" { "Error: Could not reach the destination <s3://bucket>" } else { "Info: Upload finished" }
        );
        crate::notify_hub::preview(n, result, &vars, &body)
    }
}
