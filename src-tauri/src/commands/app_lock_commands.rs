//! App lock (Settings → App Lock, the lock screen). See `app_lock`.

use crate::app_lock::{announce, AppLock, AppLockSettings, AppLockStatus};
use crate::core::error::AppError;
use crate::security::audit::SecurityAudit;
use crate::security::MasterPasswordManager;
use crate::storage::Repository;
use tauri::State;

async fn has_password(mpw: &MasterPasswordManager, repo: &Repository) -> bool {
    mpw.is_master_password_set(repo).await.unwrap_or(false)
}

#[tauri::command]
pub async fn app_lock_status(lock: State<'_, AppLock>, mpw: State<'_, MasterPasswordManager>, repo: State<'_, Repository>) -> Result<AppLockStatus, AppError> {
    Ok(lock.status(has_password(&mpw, &repo).await))
}

/// Change the lock. While it is on, every change needs the master password
/// (`password`). Turning it on needs the master password too, or, when
/// none exists yet, a new one (`new_password`, at least 8 characters).
#[tauri::command]
pub async fn app_lock_set_settings(
    settings: AppLockSettings,
    password: Option<String>,
    new_password: Option<String>,
    app: tauri::AppHandle,
    lock: State<'_, AppLock>,
    mpw: State<'_, MasterPasswordManager>,
    repo: State<'_, Repository>,
) -> Result<AppLockStatus, AppError> {
    let was = lock.settings();
    if was.enabled || settings.enabled {
        if has_password(&mpw, &repo).await {
            let ok = match password {
                Some(p) if !p.is_empty() => mpw.verify_master_password(&repo, p).await?,
                _ => false,
            };
            SecurityAudit::log_auth_attempt(repo.inner(), ok).await;
            if !ok {
                return Err(AppError::auth("Wrong master password", "Enter the master password to change the app lock."));
            }
        } else {
            match new_password {
                Some(p) => mpw.set_master_password(&repo, p).await?,
                None => return Err(AppError::validation("Choose a master password first: the app lock asks for it to unlock.")),
            }
        }
    }
    lock.save(&repo, settings).await?;
    SecurityAudit::log(repo.inner(), "app_lock.settings", "user", "app", &format!("enabled={} lock_on_start={} idle_minutes={}", settings.enabled, settings.lock_on_start, settings.idle_minutes)).await;
    announce(&app, lock.is_locked());
    Ok(lock.status(has_password(&mpw, &repo).await))
}

/// Lock every window now.
#[tauri::command]
pub async fn app_lock_now(app: tauri::AppHandle, lock: State<'_, AppLock>, repo: State<'_, Repository>) -> Result<(), AppError> {
    if !lock.lock() {
        return Err(AppError::state("The app lock is off", "Turn it on in Settings → App Lock first."));
    }
    SecurityAudit::log_lock_event(repo.inner(), true).await;
    announce(&app, true);
    Ok(())
}

/// Unlock with the master password; `false` for a wrong one. After too many
/// wrong ones the password manager refuses attempts for a while (an error).
#[tauri::command]
pub async fn app_lock_unlock(password: String, app: tauri::AppHandle, lock: State<'_, AppLock>, mpw: State<'_, MasterPasswordManager>, repo: State<'_, Repository>) -> Result<bool, AppError> {
    if !lock.is_locked() {
        return Ok(true);
    }
    let ok = mpw.verify_master_password(&repo, password).await?;
    SecurityAudit::log_auth_attempt(repo.inner(), ok).await;
    if ok {
        lock.unlock();
        SecurityAudit::log_lock_event(repo.inner(), false).await;
        announce(&app, false);
    }
    Ok(ok)
}

/// A window was used (sent at most every few seconds): the idle timer restarts.
#[tauri::command]
pub async fn app_lock_activity(lock: State<'_, AppLock>) -> Result<(), AppError> {
    lock.touch();
    Ok(())
}
