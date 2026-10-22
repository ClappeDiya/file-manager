//! Office document previews by handing the file to software on the machine:
//!
//! - macOS: Quick Look (`qlmanage -t`) renders the first page as PNG.
//! - Anywhere LibreOffice is installed: headless conversion to PDF, shown
//!   by the app's PDF viewer. It runs with its own profile directory, so it
//!   neither waits for nor disturbs a LibreOffice the user has open.
//!
//! Results are cached by path, size and modification time; the cache keeps
//! the newest renders within a byte budget. Without a renderer, callers fall
//! back to the extracted text (`ufop_core::doctext`) and "open in the default app".

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const RENDER_TIMEOUT: Duration = Duration::from_secs(90);
const CACHE_BUDGET_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OfficeRender {
    /// `pdf`, `image` or `none`.
    pub kind: String,
    pub path: Option<String>,
    /// `quicklook` or `libreoffice`.
    pub renderer: Option<String>,
    /// Why there is no render (`none`).
    pub message: Option<String>,
}

impl OfficeRender {
    fn none(message: impl Into<String>) -> Self {
        Self { kind: "none".into(), path: None, renderer: None, message: Some(message.into()) }
    }
}

/// LibreOffice's `soffice`: `UFOP_SOFFICE`, then PATH, then the usual install locations.
pub fn find_soffice() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("UFOP_SOFFICE").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let exe = if cfg!(windows) { "soffice.exe" } else { "soffice" };
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).flat_map(|d| [d.join(exe), d.join("libreoffice")]);
    let known: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec!["/Applications/LibreOffice.app/Contents/MacOS/soffice".into()]
    } else if cfg!(windows) {
        ["ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(|v| std::env::var_os(v)).map(|p| PathBuf::from(p).join("LibreOffice").join("program").join("soffice.exe")).collect()
    } else {
        vec!["/usr/bin/soffice".into(), "/usr/lib/libreoffice/program/soffice".into(), "/opt/libreoffice/program/soffice".into(), "/snap/bin/libreoffice".into()]
    };
    on_path.chain(known).find(|p| p.is_file())
}

pub struct OfficeRenderer {
    dir: PathBuf,
    soffice: Option<PathBuf>,
    quicklook: bool,
    /// One render at a time: LibreOffice locks its profile, and conversions are heavy.
    busy: tokio::sync::Mutex<()>,
}

impl OfficeRenderer {
    pub fn new(dir: impl Into<PathBuf>, soffice: Option<PathBuf>, quicklook: bool) -> Self {
        Self { dir: dir.into(), soffice, quicklook, busy: tokio::sync::Mutex::new(()) }
    }

    /// `<cache dir>/office-previews` with the renderers found on this machine.
    pub fn open_default() -> Self {
        let dir = directories::ProjectDirs::from("com", "ufop", "unified-file-ops")
            .map(|d| d.cache_dir().join("office-previews"))
            .unwrap_or_else(|| ufop_core::session::user_temp_subdir("office-previews"));
        Self::new(dir, find_soffice(), cfg!(target_os = "macos") && Path::new("/usr/bin/qlmanage").is_file())
    }

    fn key(path: &Path) -> Result<String, AppError> {
        let meta = std::fs::metadata(path)?;
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
        let mut h = Sha256::new();
        h.update(path.to_string_lossy().as_bytes());
        h.update(format!("|{}|{mtime}", meta.len()).as_bytes());
        Ok(hex::encode(&h.finalize()[..16]))
    }

    /// A PDF or image of `path`, rendered once and then served from the cache.
    pub async fn render(&self, path: &Path) -> Result<OfficeRender, AppError> {
        if !path.is_file() {
            return Err(AppError::not_found(format!("File not found: {}", path.display())));
        }
        let key = Self::key(path)?;
        let _one = self.busy.lock().await;
        for (ext, kind, renderer) in [("pdf", "pdf", "libreoffice"), ("png", "image", "quicklook")] {
            let cached = self.dir.join(format!("{key}.{ext}"));
            if cached.is_file() {
                return Ok(OfficeRender { kind: kind.into(), path: Some(cached.to_string_lossy().into_owned()), renderer: Some(renderer.into()), message: None });
            }
        }
        std::fs::create_dir_all(&self.dir)?;
        let work = self.dir.join(format!("work-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&work)?;
        let result = self.render_into(path, &key, &work).await;
        let _ = std::fs::remove_dir_all(&work);
        self.evict();
        result
    }

    async fn render_into(&self, path: &Path, key: &str, work: &Path) -> Result<OfficeRender, AppError> {
        let mut failures = Vec::new();
        if self.quicklook {
            let mut cmd = tokio::process::Command::new("/usr/bin/qlmanage");
            cmd.args(["-t", "-s", "1600", "-o"]).arg(work).arg(path);
            match run(cmd).await {
                Ok(()) => {
                    if let Some(png) = first_with_ext(work, "png") {
                        let dest = self.dir.join(format!("{key}.png"));
                        std::fs::rename(&png, &dest)?;
                        return Ok(OfficeRender { kind: "image".into(), path: Some(dest.to_string_lossy().into_owned()), renderer: Some("quicklook".into()), message: None });
                    }
                    failures.push("Quick Look produced no image".to_string());
                }
                Err(e) => failures.push(format!("Quick Look: {e}")),
            }
        }
        if let Some(soffice) = &self.soffice {
            let profile = self.dir.join("libreoffice-profile");
            let profile_url = format!("file://{}{}", if cfg!(windows) { "/" } else { "" }, profile.to_string_lossy().replace('\\', "/"));
            let mut cmd = tokio::process::Command::new(soffice);
            cmd.arg(format!("-env:UserInstallation={profile_url}"))
                .args(["--headless", "--norestore", "--nolockcheck", "--nodefault", "--convert-to", "pdf", "--outdir"])
                .arg(work)
                .arg(path);
            match run(cmd).await {
                Ok(()) => {
                    if let Some(pdf) = first_with_ext(work, "pdf") {
                        let dest = self.dir.join(format!("{key}.pdf"));
                        std::fs::rename(&pdf, &dest)?;
                        return Ok(OfficeRender { kind: "pdf".into(), path: Some(dest.to_string_lossy().into_owned()), renderer: Some("libreoffice".into()), message: None });
                    }
                    failures.push("LibreOffice could not convert this file".to_string());
                }
                Err(e) => failures.push(format!("LibreOffice: {e}")),
            }
        }
        Ok(OfficeRender::none(if failures.is_empty() {
            "No document renderer is installed. Install LibreOffice for full previews, or open the file in its app.".to_string()
        } else {
            failures.join("; ")
        }))
    }

    /// Keep the newest renders within the budget.
    fn evict(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = rd
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), e.path()))
            })
            .collect();
        files.sort_by(|a, b| b.0.cmp(&a.0));
        let mut total = 0u64;
        for (_, len, p) in files {
            total += len;
            if total > CACHE_BUDGET_BYTES {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

fn first_with_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.filter_map(Result::ok).map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)))
}

/// Run a renderer with a timeout; it is killed if it takes too long.
async fn run(mut cmd: tokio::process::Command) -> Result<(), String> {
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    match tokio::time::timeout(RENDER_TIMEOUT, child.wait_with_output()).await {
        Err(_) => Err(format!("timed out after {}s", RENDER_TIMEOUT.as_secs())),
        Ok(Err(e)) => Err(e.to_string()),
        Ok(Ok(o)) if o.status.success() => Ok(()),
        Ok(Ok(o)) => Err(String::from_utf8_lossy(&o.stderr).trim().chars().take(300).collect::<String>()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_renderer_explains_and_caches_nothing() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.docx");
        std::fs::write(&f, "x").unwrap();
        let r = OfficeRenderer::new(d.path().join("cache"), None, false);
        let out = r.render(&f).await.unwrap();
        assert_eq!(out.kind, "none");
        assert!(out.message.unwrap().contains("LibreOffice"));
        assert!(r.render(&d.path().join("missing.docx")).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uses_the_converter_and_caches_by_content() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        // A stand-in for soffice: writes <outdir>/<stem>.pdf and counts its runs.
        let fake = d.path().join("soffice");
        let count = d.path().join("runs");
        std::fs::write(
            &fake,
            format!("#!/bin/sh\necho run >> {}\nwhile [ \"$1\" != \"--outdir\" ]; do shift; done\nout=\"$2\"; src=\"$3\"\nn=$(basename \"$src\"); printf '%%PDF-1.4' > \"$out/${{n%.*}}.pdf\"\n", count.display()),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let f = d.path().join("report.docx");
        std::fs::write(&f, "v1").unwrap();
        let r = OfficeRenderer::new(d.path().join("cache"), Some(fake.clone()), false);
        let a = r.render(&f).await.unwrap();
        assert_eq!((a.kind.as_str(), a.renderer.as_deref()), ("pdf", Some("libreoffice")));
        assert_eq!(std::fs::read(a.path.as_ref().unwrap()).unwrap(), b"%PDF-1.4");
        assert_eq!(r.render(&f).await.unwrap(), a, "served from the cache");
        assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 1);
        std::fs::write(&f, "v2 changed").unwrap();
        assert_ne!(r.render(&f).await.unwrap().path, a.path, "a changed file renders again");
        // No work directories are left behind.
        assert!(std::fs::read_dir(d.path().join("cache")).unwrap().filter_map(Result::ok).all(|e| !e.file_name().to_string_lossy().starts_with("work-")));

        // A converter that fails reports why.
        std::fs::write(&fake, "#!/bin/sh\necho 'source file could not be loaded' >&2\nexit 1\n").unwrap();
        std::fs::write(&f, "v3").unwrap();
        let bad = r.render(&f).await.unwrap();
        assert_eq!(bad.kind, "none");
        assert!(bad.message.unwrap().contains("could not be loaded"));
    }

    /// Converts a real document when LibreOffice is installed (`UFOP_TEST_SOFFICE=1`).
    #[tokio::test]
    #[ignore]
    async fn real_libreoffice() {
        let (Some(soffice), Ok(sample)) = (find_soffice(), std::env::var("UFOP_TEST_OFFICE_FILE")) else { return };
        let d = tempfile::tempdir().unwrap();
        let r = OfficeRenderer::new(d.path(), Some(soffice), false);
        let out = r.render(Path::new(&sample)).await.unwrap();
        assert_eq!(out.kind, "pdf", "{out:?}");
        assert!(std::fs::read(out.path.unwrap()).unwrap().starts_with(b"%PDF"));
    }
}
