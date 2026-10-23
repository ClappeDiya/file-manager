//! Restoring file owners. Backups record the Unix owner and group ids (and
//! on Windows the security descriptor, whose `O:` part is the owner). Giving
//! a file to another user needs privileges: root or `CAP_CHOWN` on Unix,
//! `SeRestorePrivilege` on Windows. Without them, `auto` skips owners with
//! one summary warning and `always` refuses to start.

use super::meta::{self, ExtMeta};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OwnerPolicy {
    /// Restore owners when running with the privileges to do so.
    #[default]
    Auto,
    /// Restore owners; refuse to start without the privileges.
    Always,
    /// Never change owners (files belong to whoever restores them).
    Never,
}

/// Changes the owner of one path from recorded metadata; returns whether
/// anything was recorded to apply.
pub type ChownFn<'f> = dyn Fn(&Path, &ExtMeta) -> std::io::Result<bool> + Sync + 'f;

/// Owner restoring for one restore run.
pub struct OwnerRestore<'f> {
    apply: bool,
    /// `auto` without privileges counts what it skips for the summary.
    count_skips: bool,
    chown: &'f ChownFn<'f>,
    pub(crate) restored: AtomicU64,
    /// Entries with a recorded owner that were left alone (unprivileged `auto`).
    pub(crate) skipped: AtomicU64,
}

impl<'f> OwnerRestore<'f> {
    /// `privileged`: whether this process may change owners.
    pub fn new(policy: OwnerPolicy, privileged: bool, chown: &'f ChownFn<'f>) -> Result<Self, AppError> {
        if policy == OwnerPolicy::Always && !privileged {
            return Err(AppError::state(
                "Restoring file owners needs administrator rights",
                if cfg!(windows) { "Run the restore as an administrator (SeRestorePrivilege), or set 'Restore owners' to Auto or Never." } else { "Run the restore as root (or with CAP_CHOWN), or set 'Restore owners' to Auto or Never." },
            ));
        }
        Ok(Self { apply: policy != OwnerPolicy::Never && privileged, count_skips: policy == OwnerPolicy::Auto && !privileged, chown, restored: AtomicU64::new(0), skipped: AtomicU64::new(0) })
    }

    /// The real platform: [`meta::can_restore_owner`] and [`meta::set_owner`].
    pub fn system(policy: OwnerPolicy) -> Result<OwnerRestore<'static>, AppError> {
        let privileged = policy != OwnerPolicy::Never && meta::can_restore_owner();
        OwnerRestore::new(policy, privileged, &meta::set_owner)
    }

    /// Apply the recorded owner of one entry. `Err` is a metadata problem.
    pub fn apply(&self, path: &Path, m: Option<&ExtMeta>) -> Result<(), String> {
        let Some(m) = m.filter(|m| m.uid.is_some() || m.gid.is_some() || m.win_sddl.as_deref().and_then(meta::sddl_owner).is_some()) else { return Ok(()) };
        if !self.apply {
            if self.count_skips {
                self.skipped.fetch_add(1, Ordering::Relaxed);
            }
            return Ok(());
        }
        match (self.chown)(path, m) {
            Ok(true) => {
                self.restored.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Ok(false) => Ok(()),
            Err(e) => Err(format!("{}: owner not restored ({e})", path.display())),
        }
    }

    pub fn restored(&self) -> u64 {
        self.restored.load(Ordering::Relaxed)
    }

    /// The one warning for owners skipped without privileges, if any.
    pub fn summary(&self) -> Option<String> {
        let n = self.skipped.load(Ordering::Relaxed);
        (n > 0).then(|| {
            let how = if cfg!(windows) { "as an administrator" } else { "as root" };
            format!("File owners were not restored for {n} item(s): restoring owners needs running {how}. The files belong to the user who ran the restore.")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn m(uid: u32, gid: u32) -> ExtMeta {
        ExtMeta { uid: Some(uid), gid: Some(gid), ..Default::default() }
    }

    #[test]
    fn privileged_auto_maps_recorded_ids_to_chown() {
        let calls = Mutex::new(Vec::new());
        let chown = |p: &Path, e: &ExtMeta| -> std::io::Result<bool> {
            calls.lock().unwrap().push((p.to_path_buf(), e.uid, e.gid));
            Ok(true)
        };
        let o = OwnerRestore::new(OwnerPolicy::Auto, true, &chown).unwrap();
        o.apply(Path::new("/r/a"), Some(&m(1000, 100))).unwrap();
        o.apply(Path::new("/r/b"), None).unwrap();
        o.apply(Path::new("/r/c"), Some(&ExtMeta::default())).unwrap();
        assert_eq!(*calls.lock().unwrap(), vec![(Path::new("/r/a").to_path_buf(), Some(1000), Some(100))]);
        assert_eq!(o.restored(), 1);
        assert_eq!(o.summary(), None);
    }

    #[test]
    fn unprivileged_auto_skips_with_one_warning_and_always_refuses() {
        let chown = |_: &Path, _: &ExtMeta| -> std::io::Result<bool> { panic!("must not be called") };
        let o = OwnerRestore::new(OwnerPolicy::Auto, false, &chown).unwrap();
        for i in 0..3 {
            o.apply(Path::new(&format!("/r/{i}")), Some(&m(0, 0))).unwrap();
        }
        let w = o.summary().unwrap();
        assert!(w.contains("3 item(s)"), "{w}");
        assert!(OwnerRestore::new(OwnerPolicy::Always, false, &chown).is_err());
        let never = OwnerRestore::new(OwnerPolicy::Never, true, &chown).unwrap();
        never.apply(Path::new("/x"), Some(&m(1, 1))).unwrap();
        assert_eq!(never.summary(), None, "never is silent");
    }

    #[test]
    fn chown_failures_are_reported() {
        let chown = |_: &Path, _: &ExtMeta| -> std::io::Result<bool> { Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)) };
        let o = OwnerRestore::new(OwnerPolicy::Always, true, &chown).unwrap();
        assert!(o.apply(Path::new("/x"), Some(&m(5, 5))).unwrap_err().contains("owner not restored"));
        assert_eq!(meta::sddl_owner("O:S-1-5-21-1G:BAD:(A;;FA;;;SY)"), Some("S-1-5-21-1"));
        assert_eq!(meta::sddl_owner("O:BAD:(A;;FA;;;SY)"), Some("BA"));
        assert_eq!(meta::sddl_owner("G:BAD:"), None);
    }

    #[cfg(unix)]
    #[test]
    fn system_owner_restore_matches_privileges() {
        // Whatever this test runs as: `always` either works or refuses.
        let privileged = meta::can_restore_owner();
        assert_eq!(OwnerRestore::system(OwnerPolicy::Always).is_ok(), privileged);
        assert!(OwnerRestore::system(OwnerPolicy::Auto).is_ok());
        // Giving a file to ourselves always works.
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        std::fs::write(&f, b"x").unwrap();
        let md = std::fs::metadata(&f).unwrap();
        let (uid, gid) = meta::owner_of(&md);
        assert!(meta::set_owner(&f, &ExtMeta { uid, gid, ..Default::default() }).unwrap());
    }
}
