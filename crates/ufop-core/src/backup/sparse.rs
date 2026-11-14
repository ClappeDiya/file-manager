//! Sparse-file writing for restores: runs of zeros are skipped with a seek
//! instead of written, so a file that had holes (virtual machine disks,
//! databases, core dumps) gets them back instead of filling the disk.
//!
//! Zero runs are detected in 4 KiB pieces aligned to the file offset (the
//! usual filesystem block size); a piece is skipped only when all of it is
//! zero. The caller sets the final length afterwards, which also covers a
//! hole at the end. On Windows the file is first marked sparse
//! (`FSCTL_SET_SPARSE`); filesystems without holes simply store the zeros.

use serde::{Deserialize, Serialize};
use std::io::{Seek, SeekFrom, Write};

/// Granularity of hole detection.
pub const CHUNK: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SparseMode {
    /// Leave holes for zero runs of 4 KiB and more.
    #[default]
    Auto,
    /// Write every byte.
    Off,
}

/// Writes data at the file's cursor, seeking over aligned all-zero pieces.
pub struct SparseWriter<'f> {
    f: &'f mut std::fs::File,
    pos: u64,
    sparse: bool,
    /// At least one hole was left.
    pub holes: bool,
}

impl<'f> SparseWriter<'f> {
    /// `sparse = false` writes everything (plain `write_all`).
    pub fn new(f: &'f mut std::fs::File, sparse: bool) -> Self {
        Self { f, pos: 0, sparse, holes: false }
    }

    pub fn position(&self) -> u64 {
        self.pos
    }

    pub fn write(&mut self, d: &[u8]) -> std::io::Result<()> {
        if !self.sparse {
            self.f.write_all(d)?;
            self.pos += d.len() as u64;
            return Ok(());
        }
        let mut i = 0usize;
        let mut run = 0usize; // start of the pending data run
        while i < d.len() {
            let n = (CHUNK - (self.pos as usize + i) % CHUNK).min(d.len() - i);
            let piece = &d[i..i + n];
            if n == CHUNK && is_zero(piece) {
                if run < i {
                    self.f.write_all(&d[run..i])?;
                }
                self.f.seek(SeekFrom::Current(n as i64))?;
                self.holes = true;
                run = i + n;
            }
            i += n;
        }
        if run < d.len() {
            self.f.write_all(&d[run..])?;
        }
        self.pos += d.len() as u64;
        Ok(())
    }
}

fn is_zero(b: &[u8]) -> bool {
    let (head, body, tail) = unsafe { b.align_to::<u128>() };
    head.iter().all(|x| *x == 0) && body.iter().all(|x| *x == 0) && tail.iter().all(|x| *x == 0)
}

/// Mark a file sparse where the filesystem needs it (NTFS). Returns whether
/// holes can be made: always true off Windows.
#[allow(unused_variables)]
pub fn prepare(f: &std::fs::File) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
        use windows_sys::Win32::System::IO::DeviceIoControl;
        let mut ret = 0u32;
        let ok = unsafe { DeviceIoControl(f.as_raw_handle() as _, FSCTL_SET_SPARSE, std::ptr::null(), 0, std::ptr::null_mut(), 0, &mut ret, std::ptr::null_mut()) };
        return ok != 0;
    }
    #[allow(unreachable_code)]
    true
}

/// Allocated bytes of a file (Unix `st_blocks * 512`), when known.
#[allow(unused_variables)]
pub fn allocated(path: &std::path::Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return std::fs::metadata(path).ok().map(|m| m.blocks() * 512);
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn write_all(parts: &[&[u8]], sparse: bool) -> (Vec<u8>, bool, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        let mut f = std::fs::File::create(&p).unwrap();
        let mut w = SparseWriter::new(&mut f, sparse);
        for part in parts {
            w.write(part).unwrap();
        }
        let (len, holes) = (w.position(), w.holes);
        f.set_len(len).unwrap();
        drop(f);
        (std::fs::read(&p).unwrap(), holes, d)
    }

    #[test]
    fn content_is_identical_with_and_without_holes() {
        let mut a = vec![7u8; 5000];
        a.extend(vec![0u8; 3 * CHUNK + 100]);
        let b = vec![0u8; 2 * CHUNK]; // a whole block of zeros
        let c = [1u8, 2, 3];
        let tail = vec![0u8; CHUNK * 2]; // hole at the end, covered by set_len
        let want: Vec<u8> = [a.as_slice(), &b, &c, &tail].concat();
        let (plain, h0, _d0) = write_all(&[&a, &b, &c, &tail], false);
        let (sparse, h1, _d1) = write_all(&[&a, &b, &c, &tail], true);
        assert_eq!(plain, want);
        assert_eq!(sparse, want);
        assert!(!h0 && h1);
        // Zeros shorter than one aligned piece are written.
        let (s, holes, _d) = write_all(&[&vec![0u8; CHUNK - 1], &[9]], true);
        assert_eq!(s.len(), CHUNK);
        assert!(!holes);
        assert!(is_zero(&[0; 33]) && !is_zero(&[0, 0, 1]));
    }

    #[cfg(unix)]
    #[test]
    fn large_hole_uses_less_disk() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("big");
        let mut f = std::fs::File::create(&p).unwrap();
        let mut w = SparseWriter::new(&mut f, true);
        w.write(b"start").unwrap();
        w.write(&vec![0u8; 32 << 20]).unwrap();
        w.write(b"end").unwrap();
        let len = w.position();
        f.set_len(len).unwrap();
        f.sync_all().unwrap();
        drop(f);
        let data = std::fs::read(&p).unwrap();
        assert_eq!(data.len() as u64, len);
        assert_eq!(&data[..5], b"start");
        assert_eq!(&data[data.len() - 3..], b"end");
        if supports_holes(d.path()) {
            let a = allocated(&p).unwrap();
            assert!(a < len / 2, "allocated {a} of {len}");
        } else {
            // Filesystems without holes (some overlay or network mounts) store the zeros.
            eprintln!("filesystem keeps no holes; content checked only");
        }
    }

    /// Whether files in `dir` can have holes (a truncated-up file stays unallocated).
    #[cfg(unix)]
    pub(crate) fn supports_holes(dir: &std::path::Path) -> bool {
        let p = dir.join(".hole-probe");
        let ok = std::fs::File::create(&p).and_then(|f| f.set_len(8 << 20).and_then(|_| f.sync_all())).is_ok() && allocated(&p).is_some_and(|a| a < 1 << 20);
        let _ = std::fs::remove_file(&p);
        ok
    }
}
