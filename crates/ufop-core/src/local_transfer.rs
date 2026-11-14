//! Local file transfers used by the CLI.
//! Each file is staged and verified before publication. A move removes originals
//! only after every file has been copied and verified.
use crate::{error::AppError, sync::copier, sync_types::CopyOptions};
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Copy, Default)]
pub struct TransferOptions {
    pub overwrite: bool,
    pub verify: bool,
    pub move_source: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct TransferOutcome {
    pub bytes_transferred: u64,
    pub files_transferred: u64,
    pub files_skipped: u64,
    pub verified: bool,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Sync {
        message: message.into(),
        advice:
            "Check the source and destination. Completed copies are retained if a later file fails."
                .into(),
    }
}

/// Resolve existing ancestors before checking overlap, including links in the
/// destination's parent. The final path need not exist.
fn resolved_path(path: &Path) -> Result<PathBuf, AppError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if normalized.exists() {
        return Ok(normalized.canonicalize()?);
    }
    let parent = normalized
        .parent()
        .ok_or_else(|| invalid("Destination has no parent."))?;
    Ok(resolved_path(parent)?.join(
        normalized
            .file_name()
            .ok_or_else(|| invalid("Invalid destination."))?,
    ))
}

fn collect(
    source: &Path,
    relative: &Path,
    files: &mut Vec<PathBuf>,
    dirs: &mut Vec<PathBuf>,
) -> Result<(), AppError> {
    let path = if relative.as_os_str().is_empty() {
        source.to_path_buf()
    } else {
        source.join(relative)
    };
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(invalid(
            "Symbolic links require the advanced copy options. No transfer was started.",
        ));
    }
    if metadata.is_file() {
        files.push(relative.to_path_buf());
    } else if metadata.is_dir() {
        dirs.push(relative.to_path_buf());
        for entry in fs::read_dir(&path)? {
            collect(source, &relative.join(entry?.file_name()), files, dirs)?;
        }
    } else {
        return Err(invalid(
            "Only regular files and folders can be transferred.",
        ));
    }
    Ok(())
}

/// Compare actual bytes, including EOF. Verification never comes from a request flag.
pub fn verify_files(source: &Path, destination: &Path) -> Result<(), AppError> {
    let mut source = File::open(source)?;
    let mut destination = File::open(destination)?;
    if source.metadata()?.len() != destination.metadata()?.len() {
        return Err(invalid("Verification failed: file sizes differ."));
    }
    let mut left = [0u8; 65536];
    let mut right = [0u8; 65536];
    loop {
        let count = source.read(&mut left)?;
        let mut read = 0;
        while read < count {
            let n = destination.read(&mut right[read..count])?;
            if n == 0 {
                return Err(invalid("Verification failed: destination is incomplete."));
            }
            read += n;
        }
        if left[..count] != right[..count] {
            return Err(invalid("Verification failed: file contents differ."));
        }
        if count == 0 {
            return if destination.read(&mut right[..1])? == 0 {
                Ok(())
            } else {
                Err(invalid("Verification failed: destination has extra bytes."))
            };
        }
    }
}

struct StagedFile(PathBuf);
impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// A directory copies its contents into destination. A file copies to a named
/// file, or into destination when destination is an existing directory.
pub fn transfer(
    source: &Path,
    destination: &Path,
    options: TransferOptions,
    mut progress: impl FnMut(u64, &Path),
) -> Result<TransferOutcome, AppError> {
    if fs::symlink_metadata(source)?.file_type().is_symlink() {
        return Err(invalid("Symbolic links require the advanced copy options."));
    }
    let source = source.canonicalize()?;
    let metadata = fs::metadata(&source)?;
    let destination = if metadata.is_file() && destination.is_dir() {
        destination.join(
            source
                .file_name()
                .ok_or_else(|| invalid("Invalid source name."))?,
        )
    } else {
        destination.to_path_buf()
    };
    let destination = resolved_path(&destination)?;
    if source == destination
        || metadata.is_dir()
            && (destination.starts_with(&source) || source.starts_with(&destination))
    {
        return Err(invalid(
            "Source and destination must be separate, non-overlapping locations.",
        ));
    }
    let mut files = vec![];
    let mut directories = vec![];
    collect(&source, Path::new(""), &mut files, &mut directories)?;
    // Preflight the entire tree before creating output. Existing directory-copy
    // collisions are skipped unless overwrite was selected.
    let target_for = |relative: &PathBuf| {
        if metadata.is_file() {
            destination.clone()
        } else {
            destination.join(relative)
        }
    };
    for relative in &files {
        let target = target_for(relative);
        if let Ok(meta) = fs::symlink_metadata(&target) {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err(invalid(format!(
                    "Destination is not a regular file: {}",
                    target.display()
                )));
            }
            if metadata.is_file() && !options.overwrite {
                return Err(invalid(
                    "Destination already exists. Choose another path or use --overwrite.",
                ));
            }
        }
        if metadata.is_dir() && !resolved_path(&target)?.starts_with(&destination) {
            return Err(invalid(
                "A destination link points outside the selected folder.",
            ));
        }
    }
    for directory in &directories {
        if !resolved_path(&destination.join(directory))?.starts_with(&destination) {
            return Err(invalid(
                "A destination link points outside the selected folder.",
            ));
        }
    }
    for directory in &directories {
        fs::create_dir_all(destination.join(directory))?;
    }
    let mut outcome = TransferOutcome::default();
    for relative in &files {
        let original = if metadata.is_file() {
            source.clone()
        } else {
            source.join(relative)
        };
        let target = target_for(relative);
        if target.exists() && !options.overwrite {
            outcome.files_skipped += 1;
            continue;
        }
        let parent = target
            .parent()
            .ok_or_else(|| invalid("Destination has no parent."))?;
        fs::create_dir_all(parent)?;
        let staged = StagedFile(parent.join(format!(".ufop-{}.partial", Uuid::new_v4())));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged.0)?;
        let bytes = copier::copy_file(&original, &staged.0, &CopyOptions::default())?;
        File::open(&staged.0)?.sync_all()?;
        if options.verify || options.move_source {
            verify_files(&original, &staged.0)?;
        }
        if options.overwrite {
            fs::rename(&staged.0, &target)?;
        } else {
            // Atomic no-clobber publication. An overlapping invocation cannot
            // replace a destination created since preflight.
            if let Err(link_error) = fs::hard_link(&staged.0, &target) {
                // FAT/exFAT and some mounted filesystems cannot create links.
                // Reserve the final name exclusively and copy into that handle.
                if target.exists() {
                    return Err(link_error.into());
                }
                let mut writer = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                let published = (|| -> Result<(), AppError> {
                    std::io::copy(&mut File::open(&staged.0)?, &mut writer)?;
                    writer.sync_all()?;
                    if options.verify || options.move_source {
                        verify_files(&original, &target)?;
                    }
                    Ok(())
                })();
                drop(writer);
                if let Err(error) = published {
                    let _ = fs::remove_file(&target);
                    return Err(error);
                }
            }
        }
        outcome.files_transferred += 1;
        outcome.bytes_transferred += bytes;
        progress(outcome.bytes_transferred, &target);
    }
    outcome.verified = (options.verify || options.move_source) && outcome.files_skipped == 0;
    if options.move_source {
        if outcome.files_skipped > 0 {
            return Err(invalid("Some destination files already exist. Copies were retained and the source was not removed."));
        }
        // Recheck every published file before removing any original.
        for relative in &files {
            let original = if metadata.is_file() {
                source.clone()
            } else {
                source.join(relative)
            };
            verify_files(&original, &target_for(relative))?;
        }
        if metadata.is_dir() {
            for relative in &files {
                fs::remove_file(source.join(relative))?;
            }
            for relative in directories.iter().rev() {
                fs::remove_dir(source.join(relative))?;
            }
        } else {
            fs::remove_file(&source)?;
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn backup_and_move_preserve_real_nested_files() {
        for move_source in [false, true] {
            let temp = tempdir().unwrap();
            let source = temp.path().join("source");
            let destination = temp.path().join("destination");
            fs::create_dir_all(source.join("nested/empty")).unwrap();
            fs::write(source.join("a"), b"important").unwrap();
            fs::write(source.join("nested/b"), [0, 255, 42]).unwrap();
            let result = transfer(
                &source,
                &destination,
                TransferOptions {
                    verify: true,
                    move_source,
                    ..Default::default()
                },
                |_, _| {},
            )
            .unwrap();
            assert_eq!(fs::read(destination.join("a")).unwrap(), b"important");
            assert_eq!(
                fs::read(destination.join("nested/b")).unwrap(),
                [0, 255, 42]
            );
            assert!(destination.join("nested/empty").is_dir());
            assert_eq!(source.exists(), !move_source);
            assert!(result.verified);
            assert_eq!(result.files_transferred, 2);
        }
    }

    #[test]
    fn verification_detects_same_size_corruption() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("a"), b"original").unwrap();
        fs::write(temp.path().join("b"), b"modified").unwrap();
        assert!(verify_files(&temp.path().join("a"), &temp.path().join("b")).is_err());
    }

    #[test]
    fn nested_destination_and_colliding_move_preserve_sources() {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("a"), b"source").unwrap();
        assert!(transfer(
            &source,
            &source.join("backup"),
            Default::default(),
            |_, _| {}
        )
        .is_err());
        assert!(!source.join("backup").exists());
        let destination = temp.path().join("destination");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("a"), b"destination").unwrap();
        assert!(transfer(
            &source,
            &destination,
            TransferOptions {
                move_source: true,
                ..Default::default()
            },
            |_, _| {}
        )
        .is_err());
        assert_eq!(fs::read(source.join("a")).unwrap(), b"source");
        assert_eq!(fs::read(destination.join("a")).unwrap(), b"destination");
    }
}
