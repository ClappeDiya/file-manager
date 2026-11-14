//! PAR2 recovery data for remote volumes, via an installed `par2`
//! (par2cmdline) executable.
//!
//! For each uploaded dblock/dlist volume `V` the engine uploads `V.par2`
//! and `V.vol000+NN.par2`. When a download fails its hash check during
//! restore or compaction, the recovery files are fetched and `par2 repair`
//! reconstructs the volume. Routine verification deliberately does *not*
//! repair: it reports the corruption.

use super::Par2Options;
use crate::error::AppError;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Par2 {
    opts: Par2Options,
    tmp_root: PathBuf,
}

fn err(msg: impl Into<String>) -> AppError {
    AppError::transfer(msg.into(), "Check that par2 (par2cmdline) is installed and the redundancy setting is valid.")
}

/// Temporary working folder removed on drop.
struct Work(PathBuf);
impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Recovery files belonging to volume `name` in a remote listing.
pub fn recovery_names<'a>(name: &str, listing: impl Iterator<Item = &'a str>) -> Vec<String> {
    let base = format!("{name}.");
    listing.filter(|n| n.starts_with(&base) && n.ends_with(".par2")).map(String::from).collect()
}

impl Par2 {
    pub fn new(opts: Par2Options, tmp_root: impl Into<PathBuf>) -> Self {
        Self { opts, tmp_root: tmp_root.into() }
    }

    pub fn enabled(&self) -> bool {
        self.opts.enabled
    }

    /// Whether the configured binary runs.
    pub fn available(&self) -> bool {
        Command::new(&self.opts.binary).arg("-V").output().map(|o| o.status.success() || !o.stdout.is_empty()).unwrap_or(false)
    }

    fn work(&self) -> Result<Work, AppError> {
        let d = self.tmp_root.join(format!("ufop-par2-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d)?;
        Ok(Work(d))
    }

    fn run(&self, dir: &Path, args: &[&str]) -> Result<(), AppError> {
        let out = Command::new(&self.opts.binary)
            .args(args)
            .current_dir(dir)
            // Volumes are already processed in parallel; concurrent par2
            // processes with spinning OpenMP pools slow each other ~40x.
            .env("OMP_NUM_THREADS", "1")
            .env("OMP_WAIT_POLICY", "PASSIVE")
            .output()
            .map_err(|e| err(format!("Could not run {}: {e}", self.opts.binary)))?;
        if !out.status.success() {
            return Err(err(format!("par2 {} failed: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim())));
        }
        Ok(())
    }

    /// Create recovery files for `data` named `name`; returns (file name, bytes).
    pub fn create(&self, name: &str, data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, AppError> {
        let w = self.work()?;
        std::fs::write(w.0.join(name), data)?;
        let r = format!("-r{}", self.opts.redundancy_percent.clamp(1, 100));
        self.run(&w.0, &["create", "-q", "-q", &r, "-n1", &format!("{name}.par2"), name])?;
        let mut out = Vec::new();
        for e in std::fs::read_dir(&w.0)? {
            let p = e?.path();
            let f = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if f.ends_with(".par2") {
                out.push((f, std::fs::read(&p)?));
            }
        }
        out.sort();
        Ok(out)
    }

    /// Repair a damaged volume with its recovery files.
    pub fn repair(&self, name: &str, damaged: &[u8], recovery: &[(String, Vec<u8>)]) -> Result<Vec<u8>, AppError> {
        let w = self.work()?;
        std::fs::write(w.0.join(name), damaged)?;
        for (n, d) in recovery {
            std::fs::write(w.0.join(n), d)?;
        }
        self.run(&w.0, &["repair", "-q", "-q", &format!("{name}.par2")])?;
        Ok(std::fs::read(w.0.join(name))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p2(dir: &Path) -> Par2 {
        Par2::new(Par2Options { enabled: true, redundancy_percent: 20, binary: "par2".into() }, dir)
    }

    #[test]
    fn create_and_repair_corruption() {
        let d = tempfile::tempdir().unwrap();
        let p = p2(d.path());
        if !p.available() {
            eprintln!("par2 not installed; skipping");
            return;
        }
        let data: Vec<u8> = (0..200_000u32).map(|i| (i * 31 % 251) as u8).collect();
        let rec = p.create("vol.dblock", &data).unwrap();
        assert!(rec.iter().any(|(n, _)| n == "vol.dblock.par2"));
        assert!(rec.len() >= 2);
        let mut bad = data.clone();
        for b in bad.iter_mut().skip(1000).take(5000) {
            *b ^= 0xff;
        }
        assert_eq!(p.repair("vol.dblock", &bad, &rec).unwrap(), data);
        // Too much damage for 20 % redundancy fails cleanly.
        let mut worse = data.clone();
        for b in worse.iter_mut().take(150_000) {
            *b ^= 0xff;
        }
        assert!(p.repair("vol.dblock", &worse, &rec).is_err());
        assert!(!std::fs::read_dir(d.path()).unwrap().any(|_| true), "temp folders cleaned up");
    }

    #[test]
    fn recovery_name_matching() {
        let names = ["a.dblock", "a.dblock.par2", "a.dblock.vol000+20.par2", "ab.dblock.par2"];
        assert_eq!(recovery_names("a.dblock", names.iter().copied()), vec!["a.dblock.par2", "a.dblock.vol000+20.par2"]);
    }
}
