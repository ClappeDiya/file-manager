//! Restore into a ZIP or TAR archive instead of a folder.
//!
//! Entries are streamed: each file is read block by block through a
//! [`BlockSource`] and written straight into the archive, so nothing but
//! the archive itself (and a few unpacked volumes in memory) is ever on
//! disk. The archive is written as `<name>.partial` and renamed when it is
//! complete; a failed or cancelled export leaves nothing behind.
//!
//! | Format | Folders | Files | Links | Modes | Times | Owner |
//! |---|---|---|---|---|---|---|
//! | TAR / TAR.GZ | yes | yes | symbolic links | yes | yes | uid/gid when recorded |
//! | ZIP (deflate or stored) | yes | yes (ZIP64 when large) | as Unix link entries | yes (Unix attributes) | yes, 2-second precision, local time, 1980–2107 | no |

use super::access::{open_file, BlockSource};
use super::{EntryKind, FileEntry, FileHasher, HashAlgorithm};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    /// ZIP with Deflate compression.
    Zip,
    /// ZIP without compression (fast; for data that is already compressed).
    ZipStored,
    Tar,
    /// TAR compressed with gzip (`.tar.gz`, `.tgz`).
    TarGz,
}

impl ArchiveFormat {
    /// The format a file name asks for (`.zip`, `.tar`, `.tar.gz`, `.tgz`).
    pub fn from_path(p: &Path) -> Option<Self> {
        let n = p.file_name()?.to_string_lossy().to_ascii_lowercase();
        if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if n.ends_with(".tar") {
            Some(Self::Tar)
        } else if n.ends_with(".zip") {
            Some(Self::Zip)
        } else {
            None
        }
    }
}

/// Where an archive restore writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveTarget {
    pub path: PathBuf,
    /// `None`: from the file name's extension.
    #[serde(default)]
    pub format: Option<ArchiveFormat>,
}

impl ArchiveTarget {
    pub fn format(&self) -> Result<ArchiveFormat, AppError> {
        self.format.or_else(|| ArchiveFormat::from_path(&self.path)).ok_or_else(|| {
            AppError::file_op(format!("Cannot tell the archive type of {}", self.path.display()), "Name the file .zip, .tar, .tar.gz or .tgz, or choose a format.")
        })
    }
}

/// Writes entries of one archive.
pub trait ArchiveSink {
    fn dir(&mut self, name: &str, e: &FileEntry) -> std::io::Result<()>;
    fn file(&mut self, name: &str, e: &FileEntry, data: &mut dyn Read) -> std::io::Result<u64>;
    fn symlink(&mut self, name: &str, e: &FileEntry, target: &str) -> std::io::Result<()>;
    fn finish(self: Box<Self>) -> std::io::Result<()>;
}

struct TarSink<W: Write> {
    b: tar::Builder<W>,
}

fn tar_header(e: &FileEntry, kind: tar::EntryType, size: u64, default_mode: u32) -> tar::Header {
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(kind);
    h.set_size(size);
    h.set_mode(e.mode.map(|m| m & 0o7777).unwrap_or(default_mode));
    h.set_mtime(e.mtime.unwrap_or(0).max(0) as u64);
    h.set_uid(0);
    h.set_gid(0);
    if let Some(m) = &e.meta {
        if let Some(u) = m.uid {
            h.set_uid(u as u64);
        }
        if let Some(g) = m.gid {
            h.set_gid(g as u64);
        }
    }
    h
}

impl<W: Write> ArchiveSink for TarSink<W> {
    fn dir(&mut self, name: &str, e: &FileEntry) -> std::io::Result<()> {
        let mut h = tar_header(e, tar::EntryType::Directory, 0, 0o755);
        self.b.append_data(&mut h, format!("{name}/"), std::io::empty())
    }
    fn file(&mut self, name: &str, e: &FileEntry, data: &mut dyn Read) -> std::io::Result<u64> {
        let mut h = tar_header(e, tar::EntryType::Regular, e.size, 0o644);
        let mut counted = Counted { inner: data, n: 0 };
        self.b.append_data(&mut h, name, &mut counted)?;
        Ok(counted.n)
    }
    fn symlink(&mut self, name: &str, e: &FileEntry, target: &str) -> std::io::Result<()> {
        let mut h = tar_header(e, tar::EntryType::Symlink, 0, 0o777);
        self.b.append_link(&mut h, name, target)
    }
    fn finish(self: Box<Self>) -> std::io::Result<()> {
        self.b.into_inner()?.flush()
    }
}

/// gzip stream that must be finished explicitly (writes the trailer).
struct Gz(Option<flate2::write::GzEncoder<std::io::BufWriter<std::fs::File>>>);
impl Write for Gz {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.as_mut().ok_or_else(|| std::io::Error::other("closed"))?.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.as_mut().map_or(Ok(()), |g| g.flush())
    }
}
impl Gz {
    fn close(&mut self) -> std::io::Result<()> {
        if let Some(g) = self.0.take() {
            g.finish()?.flush()?;
        }
        Ok(())
    }
}

struct TarGzSink(TarSink<Gz>);
impl ArchiveSink for TarGzSink {
    fn dir(&mut self, name: &str, e: &FileEntry) -> std::io::Result<()> {
        self.0.dir(name, e)
    }
    fn file(&mut self, name: &str, e: &FileEntry, data: &mut dyn Read) -> std::io::Result<u64> {
        self.0.file(name, e, data)
    }
    fn symlink(&mut self, name: &str, e: &FileEntry, target: &str) -> std::io::Result<()> {
        self.0.symlink(name, e, target)
    }
    fn finish(self: Box<Self>) -> std::io::Result<()> {
        let mut gz = self.0.b.into_inner()?;
        gz.close()
    }
}

struct Counted<'r> {
    inner: &'r mut dyn Read,
    n: u64,
}
impl Read for Counted<'_> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(b)?;
        self.n += n as u64;
        Ok(n)
    }
}

struct ZipSink {
    w: zip::ZipWriter<std::io::BufWriter<std::fs::File>>,
    method: zip::CompressionMethod,
}

fn zip_time(mtime: Option<i64>) -> zip::DateTime {
    use chrono::{Datelike, Timelike};
    mtime
        .and_then(|m| chrono::DateTime::from_timestamp(m, 0))
        .map(|t| t.with_timezone(&chrono::Local))
        .and_then(|t| zip::DateTime::from_date_and_time(t.year().clamp(1980, 2107) as u16, t.month() as u8, t.day() as u8, t.hour() as u8, t.minute() as u8, t.second().min(58) as u8).ok())
        .unwrap_or_default()
}

fn zerr(e: zip::result::ZipError) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

impl ZipSink {
    fn options(&self, e: &FileEntry, default_mode: u32) -> zip::write::SimpleFileOptions {
        zip::write::SimpleFileOptions::default()
            .compression_method(self.method)
            .last_modified_time(zip_time(e.mtime))
            .unix_permissions(e.mode.map(|m| m & 0o7777).unwrap_or(default_mode))
            .large_file(e.size >= u32::MAX as u64)
    }
}

impl ArchiveSink for ZipSink {
    fn dir(&mut self, name: &str, e: &FileEntry) -> std::io::Result<()> {
        let o = self.options(e, 0o755);
        self.w.add_directory(format!("{name}/"), o).map_err(zerr)
    }
    fn file(&mut self, name: &str, e: &FileEntry, data: &mut dyn Read) -> std::io::Result<u64> {
        let o = self.options(e, 0o644);
        self.w.start_file(name, o).map_err(zerr)?;
        std::io::copy(data, &mut self.w)
    }
    fn symlink(&mut self, name: &str, e: &FileEntry, target: &str) -> std::io::Result<()> {
        let o = self.options(e, 0o777);
        self.w.add_symlink(name, target, o).map_err(zerr)
    }
    fn finish(self: Box<Self>) -> std::io::Result<()> {
        self.w.finish().map_err(zerr)?.flush()
    }
}

/// Open an archive writer for `format` at `path` (the file is created).
pub fn open_sink(path: &Path, format: ArchiveFormat) -> Result<Box<dyn ArchiveSink>, AppError> {
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    Ok(match format {
        ArchiveFormat::Zip | ArchiveFormat::ZipStored => Box::new(ZipSink {
            w: zip::ZipWriter::new(f),
            method: if format == ArchiveFormat::Zip { zip::CompressionMethod::Deflated } else { zip::CompressionMethod::Stored },
        }),
        ArchiveFormat::Tar => {
            let mut b = tar::Builder::new(f);
            b.mode(tar::HeaderMode::Complete);
            b.follow_symlinks(false);
            Box::new(TarSink { b })
        }
        ArchiveFormat::TarGz => {
            let gz = Gz(Some(flate2::write::GzEncoder::new(f, flate2::Compression::default())));
            let mut b = tar::Builder::new(gz);
            b.mode(tar::HeaderMode::Complete);
            b.follow_symlinks(false);
            Box::new(TarGzSink(TarSink { b }))
        }
    })
}

/// Result of [`write_archive`].
#[derive(Debug, Default)]
pub struct ArchiveOutcome {
    pub files: u64,
    pub folders: u64,
    pub links: u64,
    pub bytes: u64,
    /// (entry path, problem, is the data missing) for entries left out.
    pub skipped: Vec<(String, String, bool)>,
}

/// Hashes what passes through, to check each file against its recorded hash.
struct Hashing<'r> {
    inner: &'r mut dyn Read,
    h: Option<FileHasher>,
}
impl Read for Hashing<'_> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(b)?;
        if let Some(h) = self.h.as_mut() {
            h.update(&b[..n]);
        }
        Ok(n)
    }
}

/// Stream `items` (entry, name in the archive) into an archive at `target`.
/// A file whose data is not available is left out and reported; a failure
/// while writing stops the export and removes the partial archive.
/// `stop` is polled between entries (cancel / fail-fast).
pub fn write_archive(
    src: &BlockSource<'_>,
    file_hash: HashAlgorithm,
    items: &[(&FileEntry, String)],
    target: &ArchiveTarget,
    verify: bool,
    stop: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, &str),
) -> Result<ArchiveOutcome, AppError> {
    let format = target.format()?;
    if let Some(p) = target.path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(p)?;
    }
    let partial = target.path.with_file_name(format!("{}.partial", target.path.file_name().unwrap_or_default().to_string_lossy()));
    let mut sink = open_sink(&partial, format)?;
    let mut out = ArchiveOutcome::default();
    let res = (|| -> Result<(), AppError> {
        for (i, (e, name)) in items.iter().enumerate() {
            if stop() {
                return Err(AppError::state("The archive export was stopped", "Start it again when ready."));
            }
            progress(i as u64, &e.path);
            match e.kind {
                EntryKind::Dir => {
                    sink.dir(name, e)?;
                    out.folders += 1;
                }
                EntryKind::Symlink => {
                    sink.symlink(name, e, e.symlink_target.as_deref().unwrap_or_default())?;
                    out.links += 1;
                }
                EntryKind::File => {
                    let mut r = match open_file(src, e) {
                        Ok(r) => r,
                        Err(err) => {
                            out.skipped.push((e.path.clone(), err.user_message(), true));
                            continue;
                        }
                    };
                    let mut hr = Hashing { inner: &mut r, h: (verify && e.hash.is_some()).then(|| FileHasher::new(file_hash)) };
                    let n = sink.file(name, e, &mut hr).map_err(|err| AppError::file_op(format!("{}: {err}", e.path), "Check the backup with Verify, and that the archive's disk has space."))?;
                    if let (Some(h), Some(want)) = (hr.h.take(), e.hash.as_deref()) {
                        if h.finish() != want {
                            out.skipped.push((e.path.clone(), "content failed hash verification (the archive holds the damaged copy)".into(), true));
                        }
                    }
                    out.files += 1;
                    out.bytes += n;
                }
            }
        }
        Ok(())
    })();
    match res.and_then(|_| sink.finish().map_err(AppError::from)) {
        Ok(()) => {
            std::fs::rename(&partial, &target.path)?;
            Ok(out)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: EntryKind, size: u64) -> FileEntry {
        FileEntry { path: path.into(), kind, size, mtime: Some(1_700_000_000), mode: Some(0o640), hash: None, blocks: vec![], symlink_target: None, meta: None }
    }

    #[test]
    fn formats_from_names() {
        assert_eq!(ArchiveFormat::from_path(Path::new("/x/out.ZIP")), Some(ArchiveFormat::Zip));
        assert_eq!(ArchiveFormat::from_path(Path::new("a.tar")), Some(ArchiveFormat::Tar));
        assert_eq!(ArchiveFormat::from_path(Path::new("a.tar.gz")), Some(ArchiveFormat::TarGz));
        assert_eq!(ArchiveFormat::from_path(Path::new("a.tgz")), Some(ArchiveFormat::TarGz));
        assert_eq!(ArchiveFormat::from_path(Path::new("a.7z")), None);
        assert!(ArchiveTarget { path: "a.rar".into(), format: None }.format().is_err());
        assert_eq!(ArchiveTarget { path: "a.bin".into(), format: Some(ArchiveFormat::ZipStored) }.format().unwrap(), ArchiveFormat::ZipStored);
    }

    #[test]
    fn tar_sink_keeps_modes_times_and_links() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.tar");
        let mut s = open_sink(&p, ArchiveFormat::Tar).unwrap();
        let mut f = entry("/a/x.txt", EntryKind::File, 5);
        f.meta = Some(crate::backup::meta::ExtMeta { uid: Some(1234), gid: Some(99), ..Default::default() });
        s.dir("a", &entry("/a", EntryKind::Dir, 0)).unwrap();
        assert_eq!(s.file("a/x.txt", &f, &mut &b"hello"[..]).unwrap(), 5);
        s.symlink("a/l", &entry("/a/l", EntryKind::Symlink, 0), "x.txt").unwrap();
        s.finish().unwrap();
        let mut a = tar::Archive::new(std::fs::File::open(&p).unwrap());
        let got: Vec<(String, tar::EntryType, u32, u64, u64, Option<String>)> = a
            .entries()
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                let h = e.header();
                (e.path().unwrap().to_string_lossy().into_owned(), h.entry_type(), h.mode().unwrap(), h.mtime().unwrap(), h.uid().unwrap(), e.link_name().unwrap().map(|l| l.to_string_lossy().into_owned()))
            })
            .collect();
        assert_eq!(got[0].0, "a/");
        assert_eq!(got[0].1, tar::EntryType::Directory);
        assert_eq!((got[1].0.as_str(), got[1].2, got[1].3, got[1].4), ("a/x.txt", 0o640, 1_700_000_000, 1234));
        assert_eq!((got[2].1, got[2].5.as_deref()), (tar::EntryType::Symlink, Some("x.txt")));
    }
}
