//! Backup workers' read stage: open an input (a file or a stream), cut it
//! into blocks with the repository chunker, hash and encode each block and
//! hand the blocks, in order, to the coordinating thread.
//!
//! Large files (above `parallel_file_threshold`) keep the chunker on one
//! thread — it only finds boundaries, which is cheap — and spread the
//! expensive part (block hash, compression, encoding) over helper threads.
//! The blocks are re-ordered before they are sent, so the file entry is
//! byte-identical to sequential reading.
//!
//! A [`StreamSource`] is any reader stored as one virtual file (`ufop
//! backup stdin`, a job's `stdin_command`).

use super::chunker::{for_each_chunk, Chunker};
use super::{hash_bytes, meta, volume, EntryKind, FileEntry, FileHasher, HashAlgorithm};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};

pub(crate) enum Msg {
    Block { idx: usize, hash: String, flag: u8, data: Vec<u8>, raw_len: u64 },
    Done { idx: usize, size: u64, hash: String },
    Failed { idx: usize, error: String },
}

pub(crate) struct WorkerCfg {
    pub(crate) chunker: Box<dyn Chunker>,
    pub(crate) codec: super::BlockCodec,
    pub(crate) read_buf: usize,
    pub(crate) block_hash: HashAlgorithm,
    pub(crate) file_hash: HashAlgorithm,
    pub(crate) format: super::VolumeFormat,
    pub(crate) respect_locks: bool,
    pub(crate) background: bool,
    /// Pause after each block (CPU intensity below 10).
    pub(crate) pause: std::time::Duration,
    /// Inputs larger than this are encoded by `helpers` threads (0 = never).
    pub(crate) parallel_threshold: u64,
    pub(crate) helpers: usize,
}

/// What a worker reads.
pub(crate) enum Input {
    Path(PathBuf),
    /// Index into the run's stream sources.
    Stream(usize),
}

/// One item of the read queue.
pub(crate) struct Work {
    pub(crate) idx: usize,
    pub(crate) input: Input,
    pub(crate) level: u32,
    /// Size from the scan (decides parallel reading).
    pub(crate) size: u64,
}

/// A reader backed up as one virtual file.
pub struct StreamSource {
    /// Path of the virtual file in the version (see [`stream_path`]).
    pub path: String,
    pub reader: Box<dyn Read + Send>,
    /// Called after the reader hit its end; an error (e.g. the producing
    /// command exited non-zero) fails the run.
    pub finish: Option<Box<dyn FnOnce() -> Result<(), String> + Send>>,
}

impl StreamSource {
    pub fn new(path: impl Into<String>, reader: impl Read + Send + 'static) -> Self {
        Self { path: stream_path(&path.into()), reader: Box::new(reader), finish: None }
    }

    pub fn with_finish(mut self, f: impl FnOnce() -> Result<(), String> + Send + 'static) -> Self {
        self.finish = Some(Box::new(f));
        self
    }
}

/// Folder virtual stream files go in when no absolute path is given.
pub const STREAM_FOLDER: &str = "/stdin";

/// `name` → `/stdin/name`; an absolute `/a/b` path is used as it is.
pub fn stream_path(name: &str) -> String {
    let n = name.trim().replace('\\', "/");
    if n.starts_with('/') {
        n
    } else {
        format!("{STREAM_FOLDER}/{}", n.trim_start_matches("./"))
    }
}

/// Entries for a virtual file: its folders (0755) and the file (0644),
/// all timestamped now.
pub(crate) fn stream_entries(path: &str, skip_metadata: bool) -> Result<Vec<FileEntry>, crate::error::AppError> {
    let p = Path::new(path);
    if p.file_name().is_none() || path.ends_with('/') || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(crate::error::AppError::validation(format!("'{path}' is not a valid name for a stream backup")));
    }
    let now = chrono::Utc::now().timestamp();
    let entry = |path: &Path, kind: EntryKind, mode: u32| FileEntry {
        path: path.to_string_lossy().replace('\\', "/"),
        kind,
        size: 0,
        mtime: (!skip_metadata).then_some(now),
        mode: (!skip_metadata).then_some(mode),
        hash: None,
        blocks: Vec::new(),
        symlink_target: None,
        meta: None,
    };
    let mut out: Vec<FileEntry> = p.ancestors().skip(1).filter(|a| a.parent().is_some()).map(|a| entry(a, EntryKind::Dir, 0o040755)).collect();
    out.reverse();
    out.push(entry(p, EntryKind::File, 0o100644));
    Ok(out)
}

/// Read one queued input and send its blocks; `Err` = the input failed.
pub(crate) fn read_input(
    cfg: &WorkerCfg,
    w: &Work,
    streams: &[Mutex<Option<StreamSource>>],
    tx: &mpsc::SyncSender<Msg>,
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<(), String> {
    match &w.input {
        Input::Path(path) => {
            let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
            if cfg.respect_locks && meta::is_advisory_locked(&f) {
                return Err("locked by another program (advisory lock)".into());
            }
            let mut f = std::io::BufReader::with_capacity(cfg.read_buf.max(cfg.chunker.max_size()), f);
            let (size, hash) = read_reader(cfg, w.idx, &mut f, w.level, w.size, tx, stop)?;
            done(w.idx, size, hash, tx)
        }
        Input::Stream(i) => {
            let src = streams.get(*i).and_then(|m| m.lock().unwrap_or_else(|e| e.into_inner()).take()).ok_or("stream already read")?;
            let StreamSource { mut reader, finish, .. } = src;
            let mut r = std::io::BufReader::with_capacity(cfg.read_buf.max(cfg.chunker.max_size()), &mut reader);
            // Size is unknown in advance: always worth spreading.
            let res = read_reader(cfg, w.idx, &mut r, w.level, u64::MAX, tx, stop);
            drop(r);
            drop(reader);
            let (size, hash) = res?;
            // The producer's verdict comes before the file counts as done.
            if let Some(f) = finish {
                f()?;
            }
            done(w.idx, size, hash, tx)
        }
    }
}

fn done(idx: usize, size: u64, hash: String, tx: &mpsc::SyncSender<Msg>) -> Result<(), String> {
    tx.send(Msg::Done { idx, size, hash }).map_err(|_| "pipeline closed".to_string())
}

/// Send every block of `r`; returns (size, file hash).
fn read_reader(cfg: &WorkerCfg, idx: usize, r: &mut dyn Read, level: u32, size_hint: u64, tx: &mpsc::SyncSender<Msg>, stop: &(dyn Fn() -> bool + Sync)) -> Result<(u64, String), String> {
    let parallel = cfg.parallel_threshold > 0 && cfg.helpers > 1 && size_hint > cfg.parallel_threshold;
    let (size, hash) = if parallel { read_parallel(cfg, idx, r, level, tx, stop)? } else { read_sequential(cfg, idx, r, level, tx, stop)? };
    if stop() {
        return Err("cancelled".into());
    }
    Ok((size, hash))
}

fn encode(cfg: &WorkerCfg, chunk: &[u8], level: u32) -> Result<(String, u8, Vec<u8>), String> {
    let hash = hash_bytes(cfg.block_hash, chunk);
    let (flag, data) = volume::encode_block(cfg.format, cfg.codec, chunk, level).map_err(|e| e.user_message())?;
    if !cfg.pause.is_zero() {
        std::thread::sleep(cfg.pause);
    }
    Ok((hash, flag, data))
}

fn read_sequential(cfg: &WorkerCfg, idx: usize, r: &mut dyn Read, level: u32, tx: &mpsc::SyncSender<Msg>, stop: &(dyn Fn() -> bool + Sync)) -> Result<(u64, String), String> {
    let mut fh = FileHasher::new(cfg.file_hash);
    let size = for_each_chunk(r, cfg.chunker.as_ref(), |chunk| {
        if stop() {
            return Err("cancelled".into());
        }
        fh.update(chunk);
        let (hash, flag, data) = encode(cfg, chunk, level)?;
        tx.send(Msg::Block { idx, hash, flag, data, raw_len: chunk.len() as u64 }).map_err(|_| "pipeline closed".to_string())?;
        Ok(true)
    })?;
    Ok((size, fh.finish()))
}

type Encoded = Result<(String, u8, Vec<u8>, u64), String>;

/// Chunk boundaries and the file hash on this thread, block hashing and
/// encoding on `cfg.helpers` threads; blocks are sent in file order.
fn read_parallel(cfg: &WorkerCfg, idx: usize, r: &mut dyn Read, level: u32, tx: &mpsc::SyncSender<Msg>, stop: &(dyn Fn() -> bool + Sync)) -> Result<(u64, String), String> {
    let helpers = cfg.helpers.max(2);
    let mut fh = FileHasher::new(cfg.file_hash);
    let (job_tx, job_rx) = mpsc::sync_channel::<(u64, Vec<u8>)>(helpers * 2);
    let job_rx = Mutex::new(job_rx);
    std::thread::scope(|sc| -> Result<(u64, String), String> {
        let (res_tx, res_rx) = mpsc::channel::<(u64, Encoded)>();
        for _ in 0..helpers {
            let (job_rx, res_tx) = (&job_rx, res_tx.clone());
            sc.spawn(move || {
                if cfg.background {
                    meta::set_background_priority();
                }
                loop {
                    let job = job_rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                    let Ok((seq, chunk)) = job else { break };
                    let out = encode(cfg, &chunk, level).map(|(h, f, d)| (h, f, d, chunk.len() as u64));
                    if res_tx.send((seq, out)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(res_tx);
        let mut ready: BTreeMap<u64, Encoded> = BTreeMap::new();
        let mut next = 0u64;
        let emit = |ready: &mut BTreeMap<u64, Encoded>, next: &mut u64| -> Result<(), String> {
            while let Some(res) = ready.remove(next) {
                let (hash, flag, data, raw_len) = res?;
                tx.send(Msg::Block { idx, hash, flag, data, raw_len }).map_err(|_| "pipeline closed".to_string())?;
                *next += 1;
            }
            Ok(())
        };
        let mut seq = 0u64;
        let size = for_each_chunk(r, cfg.chunker.as_ref(), |chunk| {
            if stop() {
                return Err("cancelled".into());
            }
            fh.update(chunk);
            job_tx.send((seq, chunk.to_vec())).map_err(|_| "encoder stopped".to_string())?;
            seq += 1;
            while let Ok((s, res)) = res_rx.try_recv() {
                ready.insert(s, res);
            }
            emit(&mut ready, &mut next)?;
            Ok(true)
        });
        drop(job_tx);
        let size = size?;
        for (s, res) in res_rx.iter() {
            ready.insert(s, res);
            emit(&mut ready, &mut next)?;
        }
        if next != seq {
            return Err("block encoding stopped early".into());
        }
        Ok((size, fh.finish()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_paths_and_entries() {
        assert_eq!(stream_path("db.sql"), "/stdin/db.sql");
        assert_eq!(stream_path("/dumps/db.sql"), "/dumps/db.sql");
        let e = stream_entries("/dumps/pg/db.sql", false).unwrap();
        let paths: Vec<&str> = e.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["/dumps", "/dumps/pg", "/dumps/pg/db.sql"]);
        assert_eq!(e[2].mode, Some(0o100644));
        assert_eq!(e[0].kind, EntryKind::Dir);
        assert!(stream_entries("/a/../b", false).is_err());
        assert!(stream_entries("/", false).is_err());
    }
}
